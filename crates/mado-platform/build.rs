//! Compiles `swift/enclave.swift` into a static library (CryptoKit's Secure
//! Enclave API is Swift-only, spec 11 §2.1).

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=swift/enclave.swift");
    println!("cargo:rerun-if-changed=swift/ondevice.swift");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x86_64",
        _ => "arm64",
    };
    let lib = out.join("libmado_enclave.a");
    let status = Command::new("xcrun")
        .args([
            "swiftc",
            "-parse-as-library",
            "-emit-library",
            "-static",
            "-O",
        ])
        .args(["-module-name", "MadoEnclave"])
        .args(["-target", &format!("{arch}-apple-macos15.0")])
        .arg("-o")
        .arg(&lib)
        .arg("swift/enclave.swift")
        .status()
        .expect("xcrun swiftc runs");
    assert!(status.success(), "swiftc failed on swift/enclave.swift");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=mado_enclave");
    for f in ["CryptoKit", "LocalAuthentication", "Security", "Foundation"] {
        println!("cargo:rustc-link-lib=framework={f}");
    }
    // The on-device helper (spec 13 §2.1): its own executable, for macOS 26
    // and later. Development builds find it here; the release script builds
    // it into the app bundle.
    let helper = out.join("mado-ondevice");
    let built = Command::new("xcrun")
        .args(["swiftc", "-parse-as-library", "-O"])
        .args(["-target", &format!("{arch}-apple-macos26.0")])
        .arg("-o")
        .arg(&helper)
        .arg("swift/ondevice.swift")
        .status()
        .is_ok_and(|s| s.success());
    if built {
        println!("cargo:rustc-env=MADO_ONDEVICE_DEV={}", helper.display());
    } else {
        println!("cargo:warning=the on-device helper did not build (needs the macOS 26 SDK)");
        println!("cargo:rustc-env=MADO_ONDEVICE_DEV=");
    }
    // The Swift runtime ships with macOS; the toolchain's folder holds the
    // compatibility shims a static Swift library may name.
    println!("cargo:rustc-link-search=native=/usr/lib/swift");
    if let Ok(o) = Command::new("xcrun")
        .args(["--toolchain", "default", "--find", "swiftc"])
        .output()
        && let Ok(path) = String::from_utf8(o.stdout)
        && let Some(bin) = PathBuf::from(path.trim()).parent()
        && let Some(usr) = bin.parent()
    {
        println!(
            "cargo:rustc-link-search=native={}",
            usr.join("lib/swift/macosx").display()
        );
    }
}
