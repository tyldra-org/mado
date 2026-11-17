//! Password CSV files (spec 11 §10): import from Chrome, Edge, Brave,
//! Safari, Firefox, 1Password and Bitwarden, detected by the header row,
//! and a Chrome-compatible export with an `otpauth` column.

use mado_vault::item::Login;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Chrome,
    Safari,
    Firefox,
    OnePassword,
    Bitwarden,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::Safari => "safari",
            Self::Firefox => "firefox",
            Self::OnePassword => "onePassword",
            Self::Bitwarden => "bitwarden",
        }
    }
}

/// One login read from a file, and when its password last changed (Unix
/// ms) when the file says.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub login: Login,
    pub changed_at: Option<i64>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CsvError {
    #[error("the file is not a password export Mado knows")]
    UnknownFormat,
    #[error("{0}")]
    Malformed(String),
}

fn detect(header: &[String]) -> Option<Format> {
    let h: Vec<String> = header
        .iter()
        .map(|s| s.trim().trim_start_matches('\u{feff}').to_ascii_lowercase())
        .collect();
    let has = |names: &[&str]| names.iter().all(|n| h.iter().any(|x| x == n));
    if has(&["login_uri", "login_username", "login_password"]) {
        Some(Format::Bitwarden)
    } else if has(&[
        "url",
        "username",
        "password",
        "httprealm",
        "formactionorigin",
    ]) {
        Some(Format::Firefox)
    } else if has(&[
        "title", "url", "username", "password", "otpauth", "favorite", "archived",
    ]) {
        Some(Format::OnePassword)
    } else if has(&["title", "url", "username", "password"]) {
        Some(Format::Safari)
    } else if has(&["name", "url", "username", "password"]) {
        Some(Format::Chrome)
    } else {
        None
    }
}

/// Reads a password CSV.
pub fn import(text: &str) -> Result<(Format, Vec<Row>), CsvError> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(text.as_bytes());
    let header: Vec<String> = reader
        .headers()
        .map_err(|e| CsvError::Malformed(e.to_string()))?
        .iter()
        .map(str::to_owned)
        .collect();
    let format = detect(&header).ok_or(CsvError::UnknownFormat)?;
    let index = |name: &str| {
        header.iter().position(|h| {
            h.trim()
                .trim_start_matches('\u{feff}')
                .eq_ignore_ascii_case(name)
        })
    };
    let col = |rec: &csv::StringRecord, name: &str| -> String {
        index(name)
            .and_then(|i| rec.get(i))
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    let opt = |s: String| (!s.is_empty()).then_some(s);
    let mut rows = Vec::new();
    for rec in reader.records() {
        let rec = rec.map_err(|e| CsvError::Malformed(e.to_string()))?;
        let (title, url, username, password, totp, notes, changed_at) = match format {
            Format::Chrome => (
                col(&rec, "name"),
                col(&rec, "url"),
                col(&rec, "username"),
                col(&rec, "password"),
                String::new(),
                col(&rec, "note"),
                None,
            ),
            Format::Safari => (
                col(&rec, "title"),
                col(&rec, "url"),
                col(&rec, "username"),
                col(&rec, "password"),
                col(&rec, "otpauth"),
                col(&rec, "notes"),
                None,
            ),
            Format::OnePassword => (
                col(&rec, "title"),
                col(&rec, "url"),
                col(&rec, "username"),
                col(&rec, "password"),
                col(&rec, "otpauth"),
                col(&rec, "notes"),
                None,
            ),
            Format::Firefox => (
                String::new(),
                col(&rec, "url"),
                col(&rec, "username"),
                col(&rec, "password"),
                String::new(),
                String::new(),
                col(&rec, "timePasswordChanged").parse().ok(),
            ),
            Format::Bitwarden => {
                if !matches!(col(&rec, "type").as_str(), "" | "login") {
                    continue;
                }
                (
                    col(&rec, "name"),
                    col(&rec, "login_uri"),
                    col(&rec, "login_username"),
                    col(&rec, "login_password"),
                    col(&rec, "login_totp"),
                    col(&rec, "notes"),
                    None,
                )
            }
        };
        if password.is_empty() && username.is_empty() {
            continue;
        }
        // Bitwarden lists several URIs separated by commas.
        let urls = url
            .split(',')
            .map(str::trim)
            .filter(|u| !u.is_empty())
            .map(str::to_owned)
            .collect();
        rows.push(Row {
            login: Login {
                title,
                urls,
                username,
                password,
                totp: opt(totp),
                notes: opt(notes),
                password_history: Vec::new(),
            },
            changed_at,
        });
    }
    Ok((format, rows))
}

/// `name,url,username,password,otpauth,note` (11 §10).
pub fn export(logins: &[Login]) -> String {
    let mut w = csv::Writer::from_writer(Vec::new());
    let _ = w.write_record(["name", "url", "username", "password", "otpauth", "note"]);
    for l in logins {
        let _ = w.write_record([
            l.title.as_str(),
            l.urls.first().map_or("", String::as_str),
            l.username.as_str(),
            l.password.as_str(),
            l.totp.as_deref().unwrap_or(""),
            l.notes.as_deref().unwrap_or(""),
        ]);
    }
    String::from_utf8(w.into_inner().unwrap_or_default()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_export_reads_back_as_chrome() {
        let l = Login {
            title: "Example".into(),
            urls: vec!["https://example.com/".into()],
            username: "me".into(),
            password: "p,w\"x".into(),
            totp: Some("otpauth://totp/x?secret=JBSWY3DP".into()),
            ..Login::default()
        };
        let text = export(std::slice::from_ref(&l));
        assert!(text.starts_with("name,url,username,password,otpauth,note\n"));
        let (format, rows) = import(&text).expect("reads");
        assert_eq!(format, Format::Chrome);
        assert_eq!(
            (
                rows[0].login.username.as_str(),
                rows[0].login.password.as_str()
            ),
            ("me", "p,w\"x")
        );
        assert!(matches!(import("a,b\n1,2"), Err(CsvError::UnknownFormat)));
    }
}
