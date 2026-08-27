//! One-shot compact reference.
//!
//! The skill that ships with this tool stays deliberately small; an agent that
//! wants the whole surface at once runs this instead of loading a large document
//! it mostly will not use (`docs/adr/0006-agent-surface.md`).

use std::io::Write;

use clap::Args;

use crate::exit::ExitCode;

#[derive(Debug, Args)]
pub struct CheatsheetArgs {
    /// Narrow the sheet to one topic: issue, auth, queue, project, goal, attachment, format.
    pub topic: Option<String>,
}

const SHEET: &str = include_str!("../../docs/cheatsheet.txt");

#[must_use]
pub fn run(args: &CheatsheetArgs) -> ExitCode {
    let mut out = anstream::stdout();

    // The sheet is compiled in from a text file, and a Windows checkout may have
    // rewritten its line endings. Normalising here keeps the section splitting
    // below platform-independent regardless of how the source was checked out.
    let sheet = SHEET.replace("\r\n", "\n");

    let Some(topic) = args.topic.as_deref() else {
        let _ = write!(out, "{sheet}");
        return ExitCode::Success;
    };

    // Sections are separated by a blank line and start with `## <topic>`.
    let wanted = format!("## {topic}");
    let mut found = false;
    for block in sheet.split("\n\n") {
        if block.starts_with(&wanted) {
            let _ = writeln!(out, "{}", block.trim_end());
            found = true;
        }
    }

    if found {
        ExitCode::Success
    } else {
        let mut err = anstream::stderr();
        let _ = writeln!(
            err,
            "unknown topic `{topic}`; run `ytcli cheatsheet` for all"
        );
        ExitCode::Failure
    }
}
