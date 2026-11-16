//! Time-based one-time codes (RFC 6238, spec 11 §7): `otpauth://totp/`
//! URIs or bare base32 secrets.

use hmac::{Hmac, KeyInit, Mac};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    Sha1,
    Sha256,
    Sha512,
}

/// A code's parameters.
#[derive(Clone, PartialEq, Eq)]
pub struct Totp {
    pub secret: Vec<u8>,
    pub algorithm: Algorithm,
    pub digits: u32,
    /// Seconds per code.
    pub period: u64,
    pub issuer: Option<String>,
    pub account: Option<String>,
}

impl std::fmt::Debug for Totp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Totp")
            .field("algorithm", &self.algorithm)
            .field("digits", &self.digits)
            .field("period", &self.period)
            .field("issuer", &self.issuer)
            .finish_non_exhaustive()
    }
}

fn base32(s: &str) -> Option<Vec<u8>> {
    let clean: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '=')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if clean.is_empty() {
        return None;
    }
    data_encoding::BASE32_NOPAD.decode(clean.as_bytes()).ok()
}

impl Totp {
    /// Reads an `otpauth://totp/Issuer:account?secret=…` URI or a bare
    /// base32 secret (defaults: SHA-1, 6 digits, 30 seconds).
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if !text.to_ascii_lowercase().starts_with("otpauth://") {
            return Some(Self {
                secret: base32(text)?,
                algorithm: Algorithm::Sha1,
                digits: 6,
                period: 30,
                issuer: None,
                account: None,
            });
        }
        let url = url::Url::parse(text).ok()?;
        if url.host_str()? != "totp" {
            return None;
        }
        let label = url.path().trim_start_matches('/');
        let label = percent(label);
        let (label_issuer, account) = match label.split_once(':') {
            Some((i, a)) => (Some(i.trim().to_owned()), Some(a.trim().to_owned())),
            None => (None, (!label.is_empty()).then(|| label.clone())),
        };
        let q = |k: &str| {
            url.query_pairs()
                .find(|(n, _)| n.eq_ignore_ascii_case(k))
                .map(|(_, v)| v.into_owned())
        };
        let algorithm = match q("algorithm").map(|a| a.to_ascii_uppercase()).as_deref() {
            None | Some("SHA1") => Algorithm::Sha1,
            Some("SHA256") => Algorithm::Sha256,
            Some("SHA512") => Algorithm::Sha512,
            Some(_) => return None,
        };
        let digits = q("digits").map_or(Ok(6), |d| d.parse()).ok()?;
        let period = q("period").map_or(Ok(30), |p| p.parse()).ok()?;
        if !(6..=8).contains(&digits) || period == 0 || period > 3600 {
            return None;
        }
        Some(Self {
            secret: base32(&q("secret")?)?,
            algorithm,
            digits,
            period,
            issuer: q("issuer").or(label_issuer),
            account,
        })
    }

    /// The code at Unix time `seconds`.
    pub fn code(&self, seconds: u64) -> String {
        let counter = (seconds / self.period).to_be_bytes();
        macro_rules! mac {
            ($d:ty) => {{
                let mut m = <Hmac<$d> as KeyInit>::new_from_slice(&self.secret)
                    .expect("HMAC takes any key length");
                m.update(&counter);
                m.finalize().into_bytes().to_vec()
            }};
        }
        let mac = match self.algorithm {
            Algorithm::Sha1 => mac!(sha1::Sha1),
            Algorithm::Sha256 => mac!(sha2::Sha256),
            Algorithm::Sha512 => mac!(sha2::Sha512),
        };
        let offset = usize::from(mac.last().copied().unwrap_or(0) & 0x0f);
        let bin = mac
            .get(offset..offset + 4)
            .map_or(0, |b| u32::from_be_bytes([b[0] & 0x7f, b[1], b[2], b[3]]));
        let code = bin % 10u32.pow(self.digits);
        format!("{code:0width$}", width = self.digits as usize)
    }

    /// Seconds until the code at `seconds` changes.
    pub fn remaining(&self, seconds: u64) -> u64 {
        self.period - seconds % self.period
    }
}

fn percent(s: &str) -> String {
    url::form_urlencoded::parse(format!("x={s}").as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238 appendix B.
    #[test]
    fn rfc_6238_vectors() {
        let enc = |b: &[u8]| data_encoding::BASE32_NOPAD.encode(b);
        let s1 = enc(b"12345678901234567890");
        let s256 = enc(b"12345678901234567890123456789012");
        let s512 = enc(b"1234567890123456789012345678901234567890123456789012345678901234");
        let make = |s: &str, alg: &str| {
            Totp::parse(&format!(
                "otpauth://totp/Test:me?secret={s}&algorithm={alg}&digits=8"
            ))
            .expect("parses")
        };
        let cases: [(u64, &str, &str, &str); 6] = [
            (59, "94287082", "46119246", "90693936"),
            (1_111_111_109, "07081804", "68084774", "25091201"),
            (1_111_111_111, "14050471", "67062674", "99943326"),
            (1_234_567_890, "89005924", "91819424", "93441116"),
            (2_000_000_000, "69279037", "90698825", "38618901"),
            (20_000_000_000, "65353130", "77737706", "47863826"),
        ];
        for (t, a, b, c) in cases {
            assert_eq!(make(&s1, "SHA1").code(t), a, "SHA1 at {t}");
            assert_eq!(make(&s256, "SHA256").code(t), b, "SHA256 at {t}");
            assert_eq!(make(&s512, "SHA512").code(t), c, "SHA512 at {t}");
        }
    }

    #[test]
    fn uris_and_bare_secrets_parse() {
        let t = Totp::parse(
            "otpauth://totp/Example:alice%40example.com?secret=JBSWY3DPEHPK3PXP&issuer=Example",
        )
        .expect("uri");
        assert_eq!(
            (t.issuer.as_deref(), t.account.as_deref()),
            (Some("Example"), Some("alice@example.com"))
        );
        assert_eq!((t.digits, t.period, t.algorithm), (6, 30, Algorithm::Sha1));
        let bare = Totp::parse("jbsw y3dp ehpk 3pxp").expect("bare");
        assert_eq!(bare.secret, t.secret);
        assert_eq!(t.code(59).len(), 6);
        assert_eq!(t.remaining(59), 1);
        for bad in [
            "otpauth://hotp/x?secret=JBSWY3DP",
            "otpauth://totp/x?secret=JBSWY3DP&digits=12",
            "otpauth://totp/x",
            "!!!",
        ] {
            assert!(Totp::parse(bad).is_none(), "{bad}");
        }
    }
}
