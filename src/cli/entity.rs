//! Shared implementation for the entity commands.
//!
//! Projects, portfolios and goals are the same endpoint family with a different
//! type name, so they share everything but the word.

use crate::cli::write::{Gate, Intent, check};
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
                Format::Text => Ok(render::entities(&found, &session.render)),
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

/// List what a portfolio contains.
pub async fn contents(id: &str, page: u32, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    let display = session.display();
    let Ok(per_page) = u32::try_from(display.limit.max(1)) else {
        return report(&"limit is too large", ExitCode::ConfirmationRequired);
    };

    match client.entities_in(id, page.max(1), per_page).await {
        Ok(found) => {
            let rendered = match session.render.format {
                Format::Text => Ok(render::contents(&found, &session.render)),
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
                Format::Text => Ok(render::entity(&found, &session.render)),
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

/// Put an entity inside a portfolio, or take it out of one.
///
/// Two requests: the entity is read first for its version, so a portfolio that
/// somebody else moved in the meantime is refused by Tracker rather than
/// overwritten. The read also means a mistyped id fails before anything is
/// written.
pub async fn place(kind: &str, id: &str, parent: Option<&str>, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    let current = match client.entity(kind, id).await {
        Ok(entity) => entity,
        Err(error) => {
            let code = error.exit_code();
            return report(&error, code);
        }
    };

    let action = match parent {
        Some(parent) => format!("put {kind} {id} into portfolio {parent}"),
        None => format!("take {kind} {id} out of its portfolio"),
    };
    let body = serde_json::json!({
        "fields": { "parentEntity": parent }
    });
    let intent = Intent {
        action: &action,
        targets: std::slice::from_ref(&current.id),
        body: &body,
    };
    if let Gate::Stop(code) = check(&intent, session) {
        return code;
    }

    match client.place_entity(kind, id, parent, current.version).await {
        Ok(placed) => {
            let rendered = match session.render.format {
                Format::Text => Ok(render::entity(&placed, &session.render)),
                Format::JsonRaw => machine(&placed, Format::Json),
                other => machine(&placed, other),
            };
            finish(rendered)
        }
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}
