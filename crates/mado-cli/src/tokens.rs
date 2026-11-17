//! Where the CLI keeps its pairing token (spec 14 §2): the Keychain,
//! service `app.mado.control`, account `<ProfileId>/<client name>`.
//! `MADO_CLI_TOKENS` names a folder used instead, for tests.

use std::path::PathBuf;

const SERVICE: &str = "app.mado.control";

fn file(account: &str) -> Option<PathBuf> {
    std::env::var_os("MADO_CLI_TOKENS").map(|d| PathBuf::from(d).join(account.replace('/', "_")))
}

pub fn get(account: &str) -> Option<String> {
    if let Some(f) = file(account) {
        return std::fs::read_to_string(f).ok().map(|s| s.trim().to_owned());
    }
    mado_platform::keychain::get(SERVICE, account)
        .ok()
        .flatten()
        .and_then(|b| String::from_utf8(b).ok())
}

pub fn set(account: &str, token: &str) {
    if let Some(f) = file(account) {
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(f, token);
        return;
    }
    if let Err(e) = mado_platform::keychain::set(SERVICE, account, token.as_bytes()) {
        eprintln!("mado: the token was not saved in the Keychain: {e}");
    }
}

pub fn delete(account: &str) {
    if let Some(f) = file(account) {
        let _ = std::fs::remove_file(f);
        return;
    }
    let _ = mado_platform::keychain::delete(SERVICE, account);
}
