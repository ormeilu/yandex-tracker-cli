//! Entry point: parse, resolve a profile, dispatch, translate the result into an
//! exit code. Everything else lives in the library so it can be tested without
//! spawning a process.

use std::io::Write;

use clap::{CommandFactory, Parser};

use ytcli::cli::{Cli, Command, GlobalArgs, Session, render_context};
use ytcli::config::{Config, paths};
use ytcli::exit::ExitCode;

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    init_tracing(cli.global.verbose);
    run(cli).await.into()
}

async fn run(cli: Cli) -> ExitCode {
    // Completions need neither config nor credentials.
    if let Command::Completions { shell } = cli.command {
        let mut command = Cli::command();
        let name = command.get_name().to_owned();
        clap_complete::generate(shell, &mut command, name, &mut std::io::stdout());
        return ExitCode::Success;
    }
    if let Command::Cheatsheet(ref args) = cli.command {
        return ytcli::cli::cheatsheet::run(args);
    }

    let session = match build_session(&cli.global) {
        Ok(session) => session,
        Err(code) => return code,
    };

    match cli.command {
        Command::Auth(ref command) => ytcli::cli::auth::run(command, &session).await,
        Command::Issue(ref command) => ytcli::cli::issue::run(command, &session).await,
        Command::Queue(ref command) => ytcli::cli::queue::run(command, &session).await,
        Command::Project(ref command) => ytcli::cli::project::run(command, &session).await,
        Command::Goal(ref command) => ytcli::cli::goal::run(command, &session).await,
        Command::Attachment(ref command) => ytcli::cli::attachment::run(command, &session).await,
        Command::Cheatsheet(_) | Command::Completions { .. } => ExitCode::Success,
    }
}

/// Load config and resolve the profile.
///
/// A missing or unusable profile is not fatal here: `auth status` must still be
/// able to run and explain what is wrong, which is exactly when it is needed.
fn build_session(global: &GlobalArgs) -> Result<Session, ExitCode> {
    let mut err = anstream::stderr();

    let config_file = match global.config.clone() {
        Some(path) => path,
        None => match paths::config_file() {
            Ok(path) => path,
            Err(error) => {
                let _ = writeln!(err, "error: {error}");
                return Err(ExitCode::Failure);
            }
        },
    };

    let config = match Config::load(&config_file) {
        Ok(config) => config,
        Err(error) => {
            let _ = writeln!(err, "error: {error}");
            return Err(ExitCode::Failure);
        }
    };

    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let env_profile = std::env::var("YTCLI_PROFILE").ok();
    let resolved = match config.resolve(global.profile.as_deref(), env_profile.as_deref(), &cwd) {
        Ok(resolved) => Some(resolved),
        Err(error) => {
            tracing::debug!(%error, "no profile resolved");
            None
        }
    };

    Ok(Session {
        render: render_context(global, resolved.as_ref()),
        resolved,
        config,
        global: global.clone(),
    })
}

/// Logs go to stderr so that stdout stays a clean, pipeable data channel.
fn init_tracing(verbose: u8) {
    let level = match verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let filter = tracing_subscriber::EnvFilter::try_from_env("YTCLI_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .without_time()
        .try_init();
}
