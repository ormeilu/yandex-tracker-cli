//! Organisation-wide field and template listings.
//!
//! Read-only. `queue fields` answers what a queue accepts; these answer what
//! the organisation defines, which is the question behind a field a queue does
//! not show and a template somebody else made.

use clap::{Subcommand, ValueEnum};

use crate::api::TemplateKind;
use crate::cli::{Session, emit, report};
use crate::exit::ExitCode;
use crate::render::{Format, machine, queue as render};

#[derive(Debug, Subcommand)]
pub enum FieldCommand {
    /// List every field defined in the organisation.
    #[command(long_about = crate::cli::help::md(crate::cli::help::FIELD_LIST))]
    List,
}

#[derive(Debug, Subcommand)]
pub enum TemplateCommand {
    /// List templates.
    #[command(long_about = crate::cli::help::md(crate::cli::help::TEMPLATE_LIST))]
    List {
        /// Which templates to list.
        #[arg(long, value_enum, default_value_t = Kind::Issue)]
        kind: Kind,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Kind {
    Issue,
    Comment,
}

impl From<Kind> for TemplateKind {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::Issue => Self::Issue,
            Kind::Comment => Self::Comment,
        }
    }
}

pub async fn run(command: &FieldCommand, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    let FieldCommand::List = command;
    match client.fields().await {
        Ok(fields) => finish(&fields, session, |fields| {
            render::fields(fields, &session.render)
        }),
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

pub async fn run_templates(command: &TemplateCommand, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    let TemplateCommand::List { kind } = command;
    match client.templates((*kind).into()).await {
        Ok(templates) => finish(&templates, session, |templates| {
            render::templates(templates, &session.render)
        }),
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

fn finish<T: serde::Serialize>(
    value: &[T],
    session: &Session,
    as_text: impl Fn(&[T]) -> String,
) -> ExitCode {
    let rendered = match session.render.format {
        Format::Text => Ok(as_text(value)),
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
