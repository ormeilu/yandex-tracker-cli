//! Credential storage.
//!
//! Tokens live in the OS keychain (macOS Keychain, Windows Credential Manager,
//! Secret Service on Linux) and are keyed by **account**, not by profile, so one
//! login serves every organisation that account can see.
//!
//! There is deliberately no plaintext fallback and no command that prints a
//! token to stdout: a missing keychain is an error with instructions, not a
//! silent downgrade to a file anyone can read.

const SERVICE: &str = "ytcli";

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("no token stored for account `{0}`; run `ytcli auth login --account {0}`")]
    Missing(String),
    #[error("the OS keychain is unavailable; ytcli never falls back to plaintext storage")]
    Unavailable(#[source] keyring::Error),
    #[error("keychain error")]
    Backend(#[source] keyring::Error),
}

fn entry(account: &str) -> Result<keyring::Entry, SecretError> {
    keyring::Entry::new(SERVICE, account).map_err(SecretError::Unavailable)
}

/// Fetch the OAuth token for an account.
pub fn token(account: &str) -> Result<String, SecretError> {
    match entry(account)?.get_password() {
        Ok(token) => Ok(token),
        Err(keyring::Error::NoEntry) => Err(SecretError::Missing(account.to_owned())),
        Err(err) => Err(SecretError::Backend(err)),
    }
}

/// Store (or replace) the OAuth token for an account.
pub fn store(account: &str, token: &str) -> Result<(), SecretError> {
    entry(account)?
        .set_password(token)
        .map_err(SecretError::Backend)
}

/// Remove the stored token. Removing a token that is not there is not an error.
pub fn forget(account: &str) -> Result<(), SecretError> {
    match entry(account)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(SecretError::Backend(err)),
    }
}

/// Whether a token exists, without moving the secret itself around.
#[must_use]
pub fn is_stored(account: &str) -> bool {
    token(account).is_ok()
}
