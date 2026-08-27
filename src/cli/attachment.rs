//! Attachment commands.
//!
//! Downloads write files that came from other people. The destination is always
//! explicit and never inferred from a server-supplied filename, so a crafted
//! attachment name cannot decide where bytes land.

use std::path::{Path, PathBuf};

use clap::Subcommand;

use crate::cli::write::{Gate, Intent, check};
use crate::cli::{Session, emit, report};
use crate::exit::ExitCode;
use crate::render::{Format, entity as render, image, machine};

#[derive(Debug, Subcommand)]
pub enum AttachmentCommand {
    /// List the attachments of an issue.
    #[command(long_about = crate::cli::help::ATTACHMENT_LIST)]
    List { key: String },
    /// Download one attachment.
    #[command(long_about = crate::cli::help::ATTACHMENT_DOWNLOAD)]
    Download {
        key: String,
        attachment: String,
        /// Directory to write into.
        #[arg(long, short = 'o')]
        out: PathBuf,
        /// Overwrite a file that is already there.
        #[arg(long)]
        force: bool,
    },
    /// Draw an image attachment in the terminal.
    #[command(long_about = crate::cli::help::ATTACHMENT_SHOW)]
    Show { key: String, attachment: String },
    /// Upload a file to an issue.
    #[command(long_about = crate::cli::help::ATTACHMENT_UPLOAD)]
    Upload { key: String, file: PathBuf },
}

pub async fn run(command: &AttachmentCommand, session: &Session) -> ExitCode {
    match command {
        AttachmentCommand::List { key } => list(key, session).await,
        AttachmentCommand::Download {
            key,
            attachment,
            out,
            force,
        } => download(key, attachment, out, *force, session).await,
        AttachmentCommand::Show { key, attachment } => show(key, attachment, session).await,
        AttachmentCommand::Upload { key, file } => upload(key, file, session).await,
    }
}

async fn list(target: &str, session: &Session) -> ExitCode {
    let (client, key) = match session.client_for(target) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let key = key.as_str();

    match client.attachments(key).await {
        Ok(attachments) => {
            let rendered = match session.render.format {
                Format::Text => Ok(render::attachments(key, &attachments, &session.render)),
                Format::JsonRaw => machine(&attachments, Format::Json),
                other => machine(&attachments, other),
            };
            match rendered {
                Ok(text) => {
                    emit(&text);
                    ExitCode::Success
                }
                Err(error) => report(&error, ExitCode::Failure),
            }
        }
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

/// Strip everything that could steer a path out of the destination directory.
///
/// The filename comes from whoever uploaded the file. It decides only the *name*
/// inside a directory the caller named explicitly, never the directory itself,
/// and a name that survives this as empty is replaced rather than trusted.
fn safe_filename(raw: &str, fallback: &str) -> String {
    let base = raw
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(raw)
        .trim()
        .trim_matches('.');

    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .collect();

    if cleaned.is_empty() {
        fallback.to_owned()
    } else {
        cleaned
    }
}

async fn download(
    target: &str,
    attachment: &str,
    out: &Path,
    force: bool,
    session: &Session,
) -> ExitCode {
    let (client, key) = match session.client_for(target) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let key = key.as_str();

    let attachments = match client.attachments(key).await {
        Ok(attachments) => attachments,
        Err(error) => {
            let code = error.exit_code();
            return report(&error, code);
        }
    };

    let Some(found) = attachments
        .iter()
        .find(|candidate| candidate.id == attachment || candidate.name == attachment)
    else {
        return report(
            &format!("issue {key} has no attachment `{attachment}`"),
            ExitCode::NotFound,
        );
    };

    let Some(url) = found.content.as_deref() else {
        return report(
            &format!("attachment `{attachment}` has no download URL"),
            ExitCode::ApiRejected,
        );
    };

    let destination = out.join(safe_filename(&found.name, &found.id));
    if destination.exists() && !force {
        return report(
            &format!(
                "{} already exists; pass --force to overwrite",
                destination.display()
            ),
            ExitCode::ConfirmationRequired,
        );
    }

    let bytes = match client.download(url).await {
        Ok(bytes) => bytes,
        Err(error) => {
            let code = error.exit_code();
            return report(&error, code);
        }
    };

    if let Err(error) = std::fs::create_dir_all(out) {
        return report(&error, ExitCode::Failure);
    }
    if let Err(error) = std::fs::write(&destination, &bytes) {
        return report(&error, ExitCode::Failure);
    }

    emit(&format!("{}\n", destination.display()));
    ExitCode::Success
}

/// Draw an image, or say exactly what to run instead.
///
/// Every path out of here that cannot draw prints the same thing: what the file
/// is, and the `download` command that puts it somewhere openable. Silence would
/// leave a caller — an agent especially — with nothing to act on, and a
/// screenful of escape codes would be worse than either.
async fn show(target: &str, attachment: &str, session: &Session) -> ExitCode {
    let (client, key) = match session.client_for(target) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let key = key.as_str();

    let found = match find_attachment(&client, key, attachment).await {
        Ok(found) => found,
        Err(code) => return code,
    };

    // Machine formats never emit pixels. A caller that asked for JSON asked for
    // a description of the attachment, and binary in the middle of a document
    // is not a description.
    if session.render.format != Format::Text {
        return match machine(&found, session.render.format) {
            Ok(text) => {
                emit(&text);
                ExitCode::Success
            }
            Err(error) => report(&error, ExitCode::Failure),
        };
    }

    let hint = format!("  ytcli attachment download {key} {} -o .", found.id);
    let what = describe(&found);

    let Some(protocol) = image::protocol() else {
        emit(&format!(
            "{what} — this terminal cannot draw images:\n{hint}\n"
        ));
        return ExitCode::Success;
    };

    let Some(url) = found.content.as_deref() else {
        return report(
            &format!("attachment `{attachment}` has no download URL"),
            ExitCode::ApiRejected,
        );
    };

    let bytes = match client.download(url).await {
        Ok(bytes) => bytes,
        Err(error) => {
            let code = error.exit_code();
            return report(&error, code);
        }
    };

    let Some(kind) = image::Kind::of(&bytes) else {
        emit(&format!("{what} is not an image:\n{hint}\n"));
        return ExitCode::Success;
    };

    if !protocol.carries(kind) {
        emit(&format!(
            "{what} is {}, which this terminal cannot draw inline:\n{hint}\n",
            kind.name()
        ));
        return ExitCode::Success;
    }

    emit(&image::draw(
        protocol,
        &bytes,
        &found.name,
        session.render.width,
    ));
    // Under the picture, not over it: the caption belongs to what precedes it,
    // and a name printed first is a name read before there is anything to
    // attach it to.
    emit(&format!("{what}\n"));
    ExitCode::Success
}

/// The name and size, for the lines that stand in for the picture.
fn describe(found: &crate::api::models::Attachment) -> String {
    match found.size {
        Some(size) => format!("{} ({})", found.name, render::human_size(size)),
        None => found.name.clone(),
    }
}

/// The attachment named by id or by filename, or the error a caller can act on.
async fn find_attachment(
    client: &crate::api::Client,
    key: &str,
    attachment: &str,
) -> Result<crate::api::models::Attachment, ExitCode> {
    let attachments = match client.attachments(key).await {
        Ok(attachments) => attachments,
        Err(error) => {
            let code = error.exit_code();
            return Err(report(&error, code));
        }
    };

    attachments
        .into_iter()
        .find(|candidate| candidate.id == attachment || candidate.name == attachment)
        .ok_or_else(|| {
            report(
                &format!("issue {key} has no attachment `{attachment}`"),
                ExitCode::NotFound,
            )
        })
}

async fn upload(target: &str, file: &Path, session: &Session) -> ExitCode {
    let (client, key) = match session.client_for(target) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let key = key.as_str();

    let bytes = match std::fs::read(file) {
        Ok(bytes) => bytes,
        Err(error) => return report(&error, ExitCode::Failure),
    };
    let name = file.file_name().map_or_else(
        || "upload".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );

    let body = serde_json::json!({
        "file": name,
        "bytes": bytes.len(),
    });
    let targets = [key.to_owned()];
    let intent = Intent {
        action: &format!("upload {name} to {key}"),
        targets: &targets,
        body: &body,
    };
    if let Gate::Stop(code) = check(&intent, session) {
        return code;
    }

    match client.upload(key, &name, bytes).await {
        Ok(attachment) => {
            emit(&format!("{key} attachment {}\n", attachment.id));
            ExitCode::Success
        }
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The name comes from whoever uploaded the file; it must not be able to
    /// choose a directory.
    #[test]
    fn a_traversing_name_is_reduced_to_its_last_segment() {
        assert_eq!(safe_filename("../../etc/passwd", "id"), "passwd");
        assert_eq!(safe_filename("/tmp/evil.sh", "id"), "evil.sh");
        assert_eq!(safe_filename("a\\b\\c.txt", "id"), "c.txt");
    }

    #[test]
    fn a_name_that_is_only_dots_falls_back_to_the_id() {
        assert_eq!(safe_filename("..", "42"), "42");
        assert_eq!(safe_filename("   ", "42"), "42");
    }

    #[test]
    fn an_ordinary_name_is_left_alone() {
        assert_eq!(safe_filename("report.pdf", "id"), "report.pdf");
    }
}
