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

/// Environment override, for CI and containers where no keychain exists.
///
/// It is not a general escape hatch: it applies to whichever account is active,
/// so it only makes sense where exactly one identity is in play.
const TOKEN_ENV: &str = "YTCLI_TOKEN";

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("no token stored for account `{0}`; run `ytcli auth login --account {0}`")]
    Missing(String),
    // Containers and sandboxes are where this lands: a Linux image with no
    // Secret Service running, or a session sandbox that is thrown away at the
    // end. Saying only that the keychain is missing leaves the reader with no
    // next step, and the next step is not "store it in a file".
    #[error(
        "the OS keychain is unavailable; ytcli never falls back to plaintext storage.\n\
         Where there is no keychain — a container, CI, a session sandbox — put the token in the \
         environment as YTCLI_TOKEN instead. Set it as an environment variable, never as a \
         command-line argument: arguments are visible to every process on the machine"
    )]
    Unavailable(#[source] keyring::Error),
    #[error("keychain error")]
    Backend(#[source] keyring::Error),
}

/// Tokens already read in this process.
///
/// macOS asks the user to approve every keychain read, so a command that looks
/// at three profiles sharing one account must not raise three dialogs. The map
/// lives for the length of one command; nothing is written to disk.
static READ: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, String>>> =
    std::sync::OnceLock::new();

fn cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, String>> {
    READ.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn entry(account: &str) -> Result<keyring::Entry, SecretError> {
    keyring::Entry::new(SERVICE, account).map_err(SecretError::Unavailable)
}

/// Where a token came from.
///
/// Worth reporting rather than keeping to ourselves: the environment override
/// applies to every account at once, so with more than one profile configured
/// it silently makes them all the same identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Keychain,
    Environment,
}

/// Fetch the OAuth token for an account.
pub fn token(account: &str) -> Result<String, SecretError> {
    token_from(account).map(|(token, _)| token)
}

/// Fetch it, and say where it came from.
///
/// The environment wins over the keychain so that a CI run, which has no
/// keychain at all, does not have to pretend otherwise.
pub fn token_from(account: &str) -> Result<(String, Origin), SecretError> {
    if let Ok(token) = std::env::var(TOKEN_ENV)
        && !token.is_empty()
    {
        tracing::debug!("using the token from {TOKEN_ENV}");
        return Ok((token, Origin::Environment));
    }

    // A poisoned lock means another thread panicked mid-read. The cache is an
    // optimisation, so the right answer is to ask the keychain again, not to
    // fail the command.
    if let Ok(cached) = cache().lock()
        && let Some(token) = cached.get(account)
    {
        return Ok((token.clone(), Origin::Keychain));
    }

    match entry(account)?.get_password() {
        Ok(token) => {
            if let Ok(mut cached) = cache().lock() {
                cached.insert(account.to_owned(), token.clone());
            }
            Ok((token, Origin::Keychain))
        }
        Err(keyring::Error::NoEntry) => Err(SecretError::Missing(account.to_owned())),
        Err(err) => Err(SecretError::Backend(err)),
    }
}

/// Whether the environment is standing in for the keychain.
///
/// One token for every account is the right behaviour in CI and wrong
/// everywhere else, so the commands that show identity say when it applies.
#[must_use]
pub fn overridden() -> bool {
    std::env::var(TOKEN_ENV).is_ok_and(|token| !token.is_empty())
}

/// Store (or replace) the OAuth token for an account.
pub fn store(account: &str, token: &str) -> Result<(), SecretError> {
    entry(account)?
        .set_password(token)
        .map_err(SecretError::Backend)?;
    if let Ok(mut cached) = cache().lock() {
        cached.insert(account.to_owned(), token.to_owned());
    }
    Ok(())
}

/// Remove the stored token. Removing a token that is not there is not an error.
pub fn forget(account: &str) -> Result<(), SecretError> {
    if let Ok(mut cached) = cache().lock() {
        cached.remove(account);
    }
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
