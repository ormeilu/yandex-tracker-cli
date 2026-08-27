//! Shared implementation for the entity commands.
//!
//! Projects and goals are the same endpoint family with a different type name,
//! so they share everything but the word. Portfolios will slot in here too.

use crate::cli::{Session, emit, report};
use crate::exit::ExitCode;
use crate::render::{Format, entity as render, machine};

/// List entities of one kind.
pub async fn list(kind: &str, query: Option<&str>, page: u32, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    let display = session.display();
    let Ok(per_page) = u32::try_from(display.limit.max(1)) else {
        return report(&"limit is too large", ExitCode::ConfirmationRequired);
    };

    match client.entities(kind, query, page.max(1), per_page).await {
        Ok(found) => {
            let rendered = match session.render.format {
                Format::Text => Ok(render::entities(&found)),
                Format::JsonRaw => machine(&found.items, Format::Json),
                other => machine(&found.items, other),
            };
            finish(rendered)
        }
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

/// Show one entity.
pub async fn get(kind: &str, id: &str, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    match client.entity(kind, id).await {
        Ok(found) => {
            let rendered = match session.render.format {
                Format::Text => Ok(render::entity(&found, session.render.description_lines)),
                Format::JsonRaw => machine(&found, Format::Json),
                other => machine(&found, other),
            };
            finish(rendered)
        }
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

fn finish(rendered: Result<String, crate::render::RenderError>) -> ExitCode {
    match rendered {
        Ok(text) => {
            emit(&text);
            ExitCode::Success
        }
        Err(error) => report(&error, ExitCode::Failure),
    }
}
