//! The lists tool `lists.yml` runs (spec 15 §7).
//!
//!   mado-lists check <dir>                 every list in <dir> is well formed
//!   mado-lists sign <dir> <out> <serial>   check, then copy each list and its
//!                                          signature to <out>
//!   mado-lists keygen <secret-key-file>    a new key pair; prints the public key
//!
//! `sign` reads the secret key from `LISTS_SIGNING_KEY` and its password
//! from `LISTS_SIGNING_KEY_PASSWORD`; `keygen` takes the password from the
//! latter. A signature is checked against the public key built into Mado
//! before anything is written, so a key that does not match fails here.

use std::path::Path;
use std::process::ExitCode;

use mado_lists::{List, PUBLIC_KEY, Store, trusted_comment};

fn check(dir: &Path) -> Result<(), String> {
    let mut known: Vec<String> = List::ALL.iter().map(|l| l.file().to_owned()).collect();
    known.push("README.md".into());
    for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let name = entry.map_err(|e| e.to_string())?.file_name();
        let name = name.to_string_lossy();
        if !known.iter().any(|k| *k == name) {
            return Err(format!("{name}: not a list Mado reads"));
        }
    }
    for list in List::ALL {
        let bytes =
            std::fs::read(dir.join(list.file())).map_err(|e| format!("{}: {e}", list.file()))?;
        list.check(&bytes).map_err(|e| e.to_string())?;
        println!("ok {}", list.file());
    }
    Ok(())
}

fn password() -> Result<String, String> {
    std::env::var("LISTS_SIGNING_KEY_PASSWORD")
        .map_err(|_| "LISTS_SIGNING_KEY_PASSWORD is not set".to_owned())
}

fn sign(dir: &Path, out: &Path, serial: &str) -> Result<(), String> {
    let serial: u64 = serial
        .parse()
        .map_err(|_| format!("{serial:?} is not a serial"))?;
    check(dir)?;
    let secret = std::env::var("LISTS_SIGNING_KEY")
        .map_err(|_| "LISTS_SIGNING_KEY is not set".to_owned())?;
    let pass = password()?;
    let sk = minisign::SecretKeyBox::from_string(&secret)
        .and_then(|b| b.into_secret_key(Some(pass)))
        .map_err(|e| format!("the secret key: {e}"))?;
    let scratch = out.join(".verify");
    let store = Store::new(scratch.clone(), PUBLIC_KEY).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    for list in List::ALL {
        let bytes = std::fs::read(dir.join(list.file())).map_err(|e| e.to_string())?;
        let signature = minisign::sign(
            None,
            &sk,
            std::io::Cursor::new(&bytes),
            Some(&trusted_comment(list, serial)),
            None,
        )
        .map_err(|e| format!("{}: {e}", list.file()))?
        .into_string();
        store
            .install(list, &bytes, &signature)
            .map_err(|e| format!("not signed with Mado's key: {e}"))?;
        std::fs::write(out.join(list.file()), &bytes).map_err(|e| e.to_string())?;
        std::fs::write(out.join(format!("{}.minisig", list.file())), signature)
            .map_err(|e| e.to_string())?;
        println!("signed {} as {serial}", list.file());
    }
    std::fs::remove_dir_all(scratch).map_err(|e| e.to_string())
}

fn keygen(secret_out: &Path) -> Result<(), String> {
    if secret_out.exists() {
        return Err(format!("{} exists", secret_out.display()));
    }
    let keys = minisign::KeyPair::generate_encrypted_keypair(Some(password()?))
        .map_err(|e| e.to_string())?;
    let secret = keys
        .sk
        .to_box(Some("Mado lists signing key"))
        .map_err(|e| e.to_string())?
        .into_string();
    std::fs::write(secret_out, secret).map_err(|e| e.to_string())?;
    println!("{}", keys.pk.to_base64());
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["check", dir] => check(Path::new(dir)),
        ["sign", dir, out, serial] => sign(Path::new(dir), Path::new(out), serial),
        ["keygen", file] => keygen(Path::new(file)),
        _ => {
            Err("usage: mado-lists check <dir> | sign <dir> <out> <serial> | keygen <file>".into())
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mado-lists: {e}");
            ExitCode::FAILURE
        }
    }
}
