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

/// Fields a project, portfolio or goal can be given on the command line.
///
/// Deliberately few. Everything else an entity has is either a reference to
/// something the caller would have to look up first, or prose that belongs in
/// the web interface rather than in shell quoting.
#[derive(Debug, Default, clap::Args)]
pub struct Fields {
    /// Name.
    #[arg(long, short = 's')]
    pub summary: Option<String>,
    /// Description.
    #[arg(long, short = 'd')]
    pub description: Option<String>,
    /// Login of whoever owns it.
    #[arg(long)]
    pub lead: Option<String>,
    /// Start date, as 2026-09-01.
    #[arg(long)]
    pub start: Option<String>,
    /// End date, as 2026-12-31.
    #[arg(long)]
    pub end: Option<String>,
}

impl Fields {
    /// What was actually given, as Tracker's field names.
    fn body(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut fields = serde_json::Map::new();
        for (name, value) in [
            ("summary", &self.summary),
            ("description", &self.description),
            ("lead", &self.lead),
            ("start", &self.start),
            ("end", &self.end),
        ] {
            if let Some(value) = value {
                fields.insert(name.to_owned(), serde_json::json!(value));
            }
        }
        fields
    }
}

/// Create a project, portfolio or goal.
pub async fn create(kind: &str, fields: &Fields, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    let Some(summary) = fields.summary.clone() else {
        return report(
            &format!("a {kind} needs a name: --summary"),
            ExitCode::ConfirmationRequired,
        );
    };

    let body = serde_json::Value::Object(fields.body());
    let targets = [summary];
    let intent = Intent {
        action: &format!("create a {kind}"),
        targets: &targets,
        body: &body,
        always_confirm: false,
    };
    if let Gate::Stop(code) = check(&intent, session) {
        return code;
    }

    match client.create_entity(kind, &body).await {
        Ok(created) => show(&created, session),
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

/// Change the fields of one.
///
/// Two requests, like `place`: the entity is read first for its version, so a
/// change somebody else made in between is refused rather than overwritten.
pub async fn update(kind: &str, id: &str, fields: &Fields, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    let body = fields.body();
    if body.is_empty() {
        return report(
            &"nothing to change: pass --summary, --description, --lead, --start or --end",
            ExitCode::ConfirmationRequired,
        );
    }
    let body = serde_json::Value::Object(body);

    let current = match client.entity(kind, id).await {
        Ok(entity) => entity,
        Err(error) => {
            let code = error.exit_code();
            return report(&error, code);
        }
    };

    let intent = Intent {
        action: &format!("change {kind} {id}"),
        targets: std::slice::from_ref(&current.id),
        body: &body,
        always_confirm: false,
    };
    if let Gate::Stop(code) = check(&intent, session) {
        return code;
    }

    match client.update_entity(kind, id, &body, current.version).await {
        Ok(updated) => show(&updated, session),
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

/// Delete one.
///
/// `always_confirm`, like `queue create`: this is irreversible in kind rather
/// than at scale. Everything the entity grouped survives — a project holds no
/// issues of its own — but the grouping itself does not come back.
pub async fn remove(kind: &str, id: &str, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    // Read first, so the confirmation names what is about to go rather than an
    // id, and so a mistyped one fails before the gate rather than after it.
    let current = match client.entity(kind, id).await {
        Ok(entity) => entity,
        Err(error) => {
            let code = error.exit_code();
            return report(&error, code);
        }
    };

    let body = serde_json::json!({ "delete": id });
    let intent = Intent {
        action: &format!("delete {kind} `{}`", current.summary),
        targets: std::slice::from_ref(&current.id),
        body: &body,
        always_confirm: true,
    };
    if let Gate::Stop(code) = check(&intent, session) {
        return code;
    }

    match client.delete_entity(kind, id).await {
        Ok(()) => {
            emit(&format!("{kind} {id} deleted\n"));
            ExitCode::Success
        }
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

/// An entity as it now stands, after a write.
fn show(entity: &crate::api::models::Entity, session: &Session) -> ExitCode {
    let rendered = match session.render.format {
        Format::Text => Ok(render::entity(entity, &session.render)),
        Format::JsonRaw => machine(entity, Format::Json),
        other => machine(entity, other),
    };
    finish(rendered)
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
        always_confirm: false,
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
