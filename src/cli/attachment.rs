//! Attachment commands.
//!
//! Downloads write files that came from other people. The destination is always
//! explicit and never inferred from a server-supplied filename, so a crafted
//! attachment name cannot decide where bytes land.

use std::path::PathBuf;

use clap::Subcommand;

use crate::cli::{Session, not_implemented};
use crate::exit::ExitCode;

#[derive(Debug, Subcommand)]
pub enum AttachmentCommand {
    /// List the attachments of an issue.
    List { key: String },
    /// Download one attachment.
    Download {
        key: String,
        attachment: String,
        /// Directory to write into.
        #[arg(long, short = 'o')]
        out: PathBuf,
    },
    /// Upload a file to an issue.
    Upload { key: String, file: PathBuf },
}

// The dispatcher is async because the implementations landing behind it are;
// the placeholders simply do not await anything yet.
#[allow(clippy::unused_async)]
pub async fn run(command: &AttachmentCommand, _session: &Session) -> ExitCode {
    match command {
        AttachmentCommand::List { .. } => not_implemented("attachment list"),
        AttachmentCommand::Download { .. } => not_implemented("attachment download"),
        AttachmentCommand::Upload { .. } => not_implemented("attachment upload"),
    }
}
