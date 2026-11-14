//! Generic password items in the login keychain (spec 11 §2, 14 §2), by
//! service and account.

use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

use crate::PlatformError;

/// errSecItemNotFound.
const NOT_FOUND: i32 = -25300;

pub fn get(service: &str, account: &str) -> Result<Option<Vec<u8>>, PlatformError> {
    match get_generic_password(service, account) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.code() == NOT_FOUND => Ok(None),
        Err(e) => Err(PlatformError::Failed(e.to_string())),
    }
}

pub fn set(service: &str, account: &str, secret: &[u8]) -> Result<(), PlatformError> {
    set_generic_password(service, account, secret).map_err(|e| PlatformError::Failed(e.to_string()))
}

/// Deletes the item; deleting a missing item is not an error.
pub fn delete(service: &str, account: &str) -> Result<(), PlatformError> {
    match delete_generic_password(service, account) {
        Ok(()) => Ok(()),
        Err(e) if e.code() == NOT_FOUND => Ok(()),
        Err(e) => Err(PlatformError::Failed(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_round_trip_through_the_login_keychain() {
        let service = format!("app.mado.test.{}", std::process::id());
        assert_eq!(get(&service, "a").expect("reads"), None);
        set(&service, "a", b"one").expect("writes");
        set(&service, "a", b"two").expect("overwrites");
        assert_eq!(
            get(&service, "a").expect("reads").as_deref(),
            Some(&b"two"[..])
        );
        delete(&service, "a").expect("deletes");
        delete(&service, "a").expect("deleting twice is fine");
        assert_eq!(get(&service, "a").expect("reads"), None);
    }
}
