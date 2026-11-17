//! QR codes in a page snapshot (spec 11 §7): `otpauth` URIs to add as
//! two-factor codes.

/// The text of every QR code in an encoded image (PNG or JPEG).
pub fn decode(bytes: &[u8]) -> Vec<String> {
    let Ok(img) = image::load_from_memory(bytes) else {
        return Vec::new();
    };
    let mut prepared = rqrr::PreparedImage::prepare(img.to_luma8());
    prepared
        .detect_grids()
        .into_iter()
        .filter_map(|g| g.decode().ok().map(|(_, text)| text))
        .collect()
}

/// The first `otpauth://totp/` URI among the codes, with its parameters.
pub fn find_totp(bytes: &[u8]) -> Option<(String, crate::totp::Totp)> {
    decode(bytes).into_iter().find_map(|text| {
        let totp = crate::totp::Totp::parse(&text)?;
        text.to_ascii_lowercase()
            .starts_with("otpauth://totp/")
            .then_some((text, totp))
    })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    /// A QR code of `text` as a PNG, with a white margin around it.
    pub fn png(text: &str) -> Vec<u8> {
        let code = qrcode::QrCode::new(text.as_bytes()).expect("encodes");
        let img = code
            .render::<image::Luma<u8>>()
            .min_dimensions(300, 300)
            .build();
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png)
            .expect("png");
        out.into_inner()
    }

    #[test]
    fn an_otpauth_code_is_found_and_other_codes_are_not() {
        let uri = "otpauth://totp/Example:me?secret=JBSWY3DPEHPK3PXP&issuer=Example";
        let (found, totp) = super::find_totp(&png(uri)).expect("found");
        assert_eq!(found, uri);
        assert_eq!(totp.issuer.as_deref(), Some("Example"));
        assert!(super::find_totp(&png("https://example.com/")).is_none());
        assert!(super::decode(b"not an image").is_empty());
    }
}
