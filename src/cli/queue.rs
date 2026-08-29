//! Queue commands. Reads, and one write: creating a queue.

use clap::Subcommand;

use crate::cli::{Session, emit, report};
use crate::exit::ExitCode;
use crate::render::{Format, machine, queue};

#[derive(Debug, Subcommand)]
pub enum QueueCommand {
    /// List queues visible to this profile.
    #[command(long_about = crate::cli::help::md(crate::cli::help::QUEUE_LIST))]
    List,
    /// Show a queue and the defaults issues in it start with.
    #[command(long_about = crate::cli::help::md(crate::cli::help::QUEUE_GET))]
    Get {
        /// Queue key, e.g. PROJ.
        key: String,
    },
    /// Create a queue, modelled on one that already exists.
    #[command(long_about = crate::cli::help::md(crate::cli::help::QUEUE_CREATE))]
    Create {
        /// Key for the new queue, e.g. PROJ. Uppercase, and permanent.
        #[arg(long, short = 'k')]
        key: String,
        /// Human-readable name.
        #[arg(long, short = 'n')]
        name: String,
        /// Existing queue to copy the issue types, workflows and defaults from.
        #[arg(long, short = 'l', value_name = "QUEUE")]
        like: String,
        /// Queue lead. Defaults to whoever the token belongs to.
        #[arg(long)]
        lead: Option<String>,
    },
    /// List the versions a queue defines.
    #[command(long_about = crate::cli::help::md(crate::cli::help::QUEUE_VERSIONS))]
    Versions {
        /// Queue key, e.g. PROJ.
        key: String,
    },
    /// List the tags in use in a queue.
    #[command(long_about = crate::cli::help::md(crate::cli::help::QUEUE_TAGS))]
    Tags {
        /// Queue key, e.g. PROJ.
        key: String,
    },
    /// Show what changes issues in this queue on its own.
    #[command(long_about = crate::cli::help::md(crate::cli::help::QUEUE_AUTOMATION))]
    Automation {
        /// Queue key, e.g. PROJ.
        key: String,
    },
    /// Show who may do what in this queue.
    #[command(long_about = crate::cli::help::md(crate::cli::help::QUEUE_ACCESS))]
    Access {
        /// Queue key, e.g. PROJ.
        key: String,
    },
    /// List the fields this queue defines itself.
    #[command(name = "local-fields", long_about = crate::cli::help::md(crate::cli::help::QUEUE_LOCAL_FIELDS))]
    LocalFields {
        /// Queue key, e.g. PROJ.
        key: String,
    },
    /// Show a queue's fields, including custom ones and their keys.
    #[command(long_about = crate::cli::help::md(crate::cli::help::QUEUE_FIELDS))]
    Fields {
        /// Queue key, e.g. PROJ.
        key: String,
    },
}

pub async fn run(command: &QueueCommand, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    match command {
        QueueCommand::List => match client.queues().await {
            Ok(queues) => render(&queues, session, |queues| {
                queue::queues(queues, &session.render)
            }),
            Err(error) => {
                let code = error.exit_code();
                report(&error, code)
            }
        },
        QueueCommand::Get { key } => match client.queue(key).await {
            Ok(settings) => render_one(&settings, session, |settings| {
                queue::settings(settings, &session.render)
            }),
            Err(error) => {
                let code = error.exit_code();
                report(&error, code)
            }
        },
        QueueCommand::Create {
            key,
            name,
            like,
            lead,
        } => create(&client, key, name, like, lead.as_deref(), session).await,
        QueueCommand::Versions { key } => match client.queue_versions(key).await {
            Ok(versions) => render(&versions, session, |versions| {
                queue::versions(key, versions, &session.render)
            }),
            Err(error) => {
                let code = error.exit_code();
                report(&error, code)
            }
        },
        QueueCommand::Tags { key } => match client.queue_tags(key).await {
            Ok(tags) => render(&tags, session, |tags| {
                queue::tags(key, tags, &session.render)
            }),
            Err(error) => {
                let code = error.exit_code();
                report(&error, code)
            }
        },
        QueueCommand::Access { key } => match client.queue_access(key).await {
            Ok(access) => render_one(&access, session, |access| {
                queue::access(key, access, &session.render)
            }),
            Err(error) => {
                let code = error.exit_code();
                report(&error, code)
            }
        },
        QueueCommand::Automation { key } => match client.queue_automation(key).await {
            Ok(automation) => render_one(&automation, session, |automation| {
                queue::automation(key, automation, &session.render)
            }),
            Err(error) => {
                let code = error.exit_code();
                report(&error, code)
            }
        },
        QueueCommand::LocalFields { key } => match client.queue_local_fields(key).await {
            Ok(fields) => render(&fields, session, |fields| {
                queue::local_fields(key, fields, &session.render)
            }),
            Err(error) => {
                let code = error.exit_code();
                report(&error, code)
            }
        },
        QueueCommand::Fields { key } => match client.queue_fields(key).await {
            Ok(fields) => render(&fields, session, |fields| {
                queue::fields(fields, &session.render)
            }),
            Err(error) => {
                let code = error.exit_code();
                report(&error, code)
            }
        },
    }
}

/// Render either as text, through the entity's own renderer, or in whichever
/// machine format was asked for.
fn render<T: serde::Serialize>(
    value: &[T],
    session: &Session,
    as_text: impl Fn(&[T]) -> String,
) -> ExitCode {
    let rendered = match session.render.format {
        Format::Text => Ok(as_text(value)),
        // There is no upstream payload worth preserving separately here: these
        // listings are already flat, so raw and normalised would be the same
        // shape with uglier names.
        Format::JsonRaw => machine(&value, Format::Json),
        other => machine(&value, other),
    };

    match rendered {
        Ok(text) => {
            emit(&text);
            ExitCode::Success
        }
        Err(error) => report(&error, ExitCode::Failure),
    }
}

/// The same, for a command whose answer is one thing rather than a list.
fn render_one<T: serde::Serialize>(
    value: &T,
    session: &Session,
    as_text: impl Fn(&T) -> String,
) -> ExitCode {
    let rendered = match session.render.format {
        Format::Text => Ok(as_text(value)),
        Format::JsonRaw => machine(value, Format::Json),
        other => machine(value, other),
    };

    match rendered {
        Ok(text) => {
            emit(&text);
            ExitCode::Success
        }
        Err(error) => report(&error, ExitCode::Failure),
    }
}

/// Create a queue, modelled on an existing one.
///
/// A queue needs an issue type paired with a workflow and a set of resolutions,
/// and workflow ids are organisation-specific strings nobody has memorised. So
/// the shape is copied from a queue that already works rather than asked for:
/// `--like PROJ` is the difference between a command someone can run and a
/// command someone can run after reading the API reference.
async fn create(
    client: &crate::api::Client,
    key: &str,
    name: &str,
    like: &str,
    lead: Option<&str>,
    session: &Session,
) -> ExitCode {
    let blueprint = match client.queue_blueprint(like).await {
        Ok(blueprint) => blueprint,
        Err(error) => {
            let code = error.exit_code();
            return report(&error, code);
        }
    };

    let lead = match lead {
        Some(lead) => lead.to_owned(),
        None => match client.myself().await {
            Ok(user) => user.login.unwrap_or(user.id),
            Err(error) => {
                let code = error.exit_code();
                return report(&error, code);
            }
        },
    };

    let body = serde_json::json!({
        "key": key,
        "name": name,
        "lead": lead,
        "defaultType": blueprint.default_type,
        "defaultPriority": blueprint.default_priority,
        "issueTypesConfig": blueprint.issue_types,
    });

    let action = format!("create queue {key} modelled on {like}");
    let targets = [key.to_owned()];
    let intent = crate::cli::write::Intent {
        action: &action,
        targets: &targets,
        body: &body,
        // A queue key is claimed once and cannot be given back: Tracker deletes
        // a queue by hiding it, and the key stays spent. Irreversible in kind,
        // not at scale, which is the case `--yes` exists for either way.
        always_confirm: true,
    };
    if let crate::cli::write::Gate::Stop(code) = crate::cli::write::check(&intent, session) {
        return code;
    }

    match client.create_queue(&body).await {
        Ok(created) => {
            let rendered = match session.render.format {
                Format::Text => Ok(queue::settings(&created, &session.render)),
                Format::JsonRaw => machine(&created, Format::Json),
                other => machine(&created, other),
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
