//! The interactive half of `auth login`.
//!
//! Modelled on `glab auth login`: one question at a time, with the token entered
//! as a password so it never reaches the terminal, the scrollback or the shell
//! history. Everything the wizard collects can also be passed as flags, which is
//! what CI and scripts use; the wizard fills in only what was not given.
//!
//! Prompts go to stderr. A wizard that wrote to stdout would corrupt the one
//! machine-readable line the command emits.

use std::io::Write as _;

use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, Input, Password, Select};

use crate::cli::guidance;
use crate::config::OrgKind;
use crate::exit::ExitCode;

/// What the wizard could not resolve on its own.
#[derive(Debug, thiserror::Error)]
pub enum WizardError {
    #[error("cancelled")]
    Cancelled,
    #[error("could not read your answer")]
    Io(#[from] dialoguer::Error),
}

impl WizardError {
    #[must_use]
    pub fn exit_code(&self) -> ExitCode {
        match self {
            Self::Cancelled => ExitCode::ConfirmationRequired,
            Self::Io(_) => ExitCode::Failure,
        }
    }
}

fn theme() -> ColorfulTheme {
    ColorfulTheme::default()
}

/// Is anyone actually there to answer?
#[must_use]
pub fn is_interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// Print the whole of `auth login --help`'s guidance, once, before anything is
/// asked for.
///
/// It used to appear a block at a time, above the prompt each block belonged
/// to. That is too late: by the time the token prompt asks for a token, the
/// person has already gone looking for one. Both blocks up front means the
/// procedure can be followed from the top, in one place, without leaving the
/// command — and it is the same text `--help` prints, so neither can drift into
/// being the real instructions while the other rots.
pub fn introduce() {
    let mut err = anstream::stderr();
    let _ = writeln!(err, "\n{}", guidance::full());
}

/// Account name: which identity this token belongs to.
pub fn account(existing: &[String]) -> Result<String, WizardError> {
    let mut err = anstream::stderr();
    if existing.is_empty() {
        let _ = writeln!(
            err,
            "{}",
            guidance::block(
                "An account holds one token. A **profile** is an organisation seen through an account — so one account can serve several organisations."
            )
        );
    } else {
        let _ = writeln!(err, "\nAccounts you already have: {}", existing.join(", "));
    }

    let name: String = Input::with_theme(&theme())
        .with_prompt("Account name")
        .default("default".to_owned())
        .interact_text()?;

    Ok(name.trim().to_owned())
}

/// The token itself, entered as a password.
///
/// `Password` is the whole point of doing this interactively: a token typed as
/// an argument is visible in `ps` and lands in shell history, and one echoed at
/// a prompt stays in the scrollback.
pub fn token(account: &str) -> Result<String, WizardError> {
    let token = Password::with_theme(&theme())
        .with_prompt(format!("Paste the OAuth token for `{account}`"))
        .interact()?;

    let token = token.trim().to_owned();
    if token.is_empty() {
        return Err(WizardError::Cancelled);
    }
    Ok(token)
}

/// Which organisation, and which flavour it is.
///
/// "Detect" is the first option and the default: the two flavours use different
/// headers, and picking the wrong one produces a 403 that reads like a rights
/// problem. Letting the tool try both costs one request.
pub fn organisation() -> Result<(String, Option<OrgKind>), WizardError> {
    let id: String = Input::with_theme(&theme())
        .with_prompt("Organisation id")
        .interact_text()?;

    let choice = Select::with_theme(&theme())
        .with_prompt("Organisation kind")
        .default(0)
        .items([
            "Detect it for me",
            "Yandex Cloud Organization (X-Cloud-Org-Id)",
            "Yandex 360 for Business (X-Org-Id)",
        ])
        .interact()?;

    let kind = match choice {
        1 => Some(OrgKind::Cloud),
        2 => Some(OrgKind::Yandex360),
        _ => None,
    };

    Ok((id.trim().to_owned(), kind))
}

/// Profile name, defaulting to the account name.
pub fn profile(default: &str) -> Result<String, WizardError> {
    let name: String = Input::with_theme(&theme())
        .with_prompt("Profile name")
        .default(default.to_owned())
        .interact_text()?;

    Ok(name.trim().to_owned())
}

/// Default queue, chosen from the ones this token can actually see.
///
/// The list is fetched after the token is verified, so this is a pick rather
/// than a spelling test — which is the difference between a wizard and a form.
pub fn queue(available: &[String]) -> Result<Option<String>, WizardError> {
    if available.is_empty() {
        let typed: String = Input::with_theme(&theme())
            .with_prompt("Default queue (optional)")
            .allow_empty(true)
            .interact_text()?;
        let typed = typed.trim();
        return Ok((!typed.is_empty()).then(|| typed.to_owned()));
    }

    let mut items: Vec<String> = vec!["(none)".to_owned()];
    items.extend(available.iter().cloned());

    let choice = Select::with_theme(&theme())
        .with_prompt("Default queue for this profile")
        .default(0)
        .items(&items)
        .interact()?;

    Ok(if choice == 0 {
        None
    } else {
        items.get(choice).cloned()
    })
}

/// The note that says which organisation this profile is.
///
/// Optional, and empty means "leave it as it was": a re-login is about the
/// token, and clearing someone's note because they pressed Enter would be a
/// surprise. `auth edit --clear-description` is the way to remove one.
pub fn description(current: Option<&str>) -> Result<Option<String>, WizardError> {
    let typed: String = Input::with_theme(&theme())
        .with_prompt("What is this organisation? (optional)")
        .with_initial_text(current.unwrap_or_default())
        .allow_empty(true)
        .interact_text()?;

    let typed = typed.trim();
    Ok((!typed.is_empty()).then(|| typed.to_owned()))
}

/// Should this profile be the one a bare command uses?
pub fn make_default(profile: &str, current: Option<&str>) -> Result<bool, WizardError> {
    let Some(current) = current else {
        // Nothing to displace, and a config with no default is unusable.
        return Ok(true);
    };
    if current == profile {
        return Ok(true);
    }

    Ok(Confirm::with_theme(&theme())
        .with_prompt(format!(
            "Make `{profile}` the default profile? (currently `{current}`)"
        ))
        .default(false)
        .interact()?)
}
