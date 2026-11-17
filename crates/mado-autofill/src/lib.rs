//! Form model, fill sources, saving, TOTP and passkeys.
//!
//! Specification: `mado-docs/spec/11-passwords-autofill.md`. Roadmap: M4.

pub mod bitwarden;
pub mod generator;
pub mod health;
pub mod onepassword;
pub mod passkey;
pub mod qr;
pub mod runner;
pub mod source;
pub mod totp;
