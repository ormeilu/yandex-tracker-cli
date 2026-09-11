//! Entry point: parse, resolve a profile, dispatch, translate the result into an
//! exit code. Everything else lives in the library so it can be tested without
//! spawning a process.

use std::io::Write;

use clap::{CommandFactory, Parser};

use ytcli::cli::{Cli, Command, GlobalArgs, Session, render_context};
use ytcli::config::{Config, paths};
use ytcli::exit::ExitCode;

/// The stack the whole program runs on: what macOS and Linux give a main thread.
const STACK: usize = 8 * 1024 * 1024;

/// Run everything on a thread with an 8 MB stack rather than on the main thread.
///
/// Windows gives the main thread 1 MB, and clap's derive builds the whole
/// command tree in a few very large functions: in a debug build the frame that
/// adds the `wiki` subcommands is most of that megabyte on its own, and
/// `ytcli board list` overflowed before parsing its arguments. A thread with
/// the stack every other platform already has costs one spawn and makes the
/// limit the same everywhere.
fn main() -> std::process::ExitCode {
    let started = std::thread::Builder::new()
        .name("ytcli".to_owned())
        .stack_size(STACK)
        .spawn(|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            match runtime {
                Ok(runtime) => runtime.block_on(async {
                    let cli = Cli::parse();
                    init_tracing(cli.global.verbose);
                    run(cli).await
                }),
                Err(error) => {
                    let _ = writeln!(std::io::stderr(), "error: could not start: {error}");
                    ExitCode::Failure
                }
            }
        });
    match started.map(std::thread::JoinHandle::join) {
        Ok(Ok(code)) => code.into(),
        // A panic already printed its message; carry it on as the panic it was.
        Ok(Err(panic)) => std::panic::resume_unwind(panic),
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "error: could not start: {error}");
            ExitCode::Failure.into()
        }
    }
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
        Command::User(ref command) => ytcli::cli::user::run(command, &session).await,
        Command::Worklog(ref command) => ytcli::cli::worklog::run(command, &session).await,
        Command::Dict(ref command) => ytcli::cli::dict::run(command, &session).await,
        Command::Field(ref command) => ytcli::cli::field::run(command, &session).await,
        Command::Template(ref command) => ytcli::cli::field::run_templates(command, &session).await,
        Command::Project(ref command) => ytcli::cli::project::run(command, &session).await,
        Command::Board(ref command) => ytcli::cli::board::run(command, &session).await,
        Command::Sprint(ref command) => ytcli::cli::sprint::run(command, &session).await,
        Command::Component(ref command) => ytcli::cli::component::run(command, &session).await,
        Command::Link(ref command) => ytcli::cli::link::run(command, &session).await,
        Command::Bulk(ref command) => ytcli::cli::bulk::run(command, &session).await,
        Command::Portfolio(ref command) => ytcli::cli::portfolio::run(command, &session).await,
        Command::Goal(ref command) => ytcli::cli::goal::run(command, &session).await,
        Command::Attachment(ref command) => ytcli::cli::attachment::run(command, &session).await,
        Command::Wiki(ref command) => ytcli::cli::wiki::run(command, &session).await,
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
        config_file,
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
