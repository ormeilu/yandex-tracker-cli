//! Drawing an image in terminals that can draw one.
//!
//! Screenshots are most of what gets attached to a bug, and the alternative to
//! this is leaving the terminal to look at one. The risk is the opposite
//! failure: a few kilobytes of escape codes dumped into a session that cannot
//! render them is worse than never trying, so every rule here errs towards
//! printing nothing.
//!
//! **Support is decided by what the terminal says it is, never by a pattern in
//! `TERM`.** Kitty, Ghostty, `WezTerm` and iTerm2 each export a variable of their
//! own; a marker either matches exactly or the answer is no.
//!
//! **A multiplexer means no.** `TMUX` or `screen` can inherit those variables
//! from the terminal they were started in while passing none of the graphics
//! through, which is exactly how the escape codes end up on screen as text.
//!
//! **The protocol decides the formats, not us.** Kitty's direct transmission
//! takes PNG only; iTerm2's takes whatever it can decode. Anything else falls
//! back to the filename and the download command.

use std::fmt::Write as _;
use std::io::IsTerminal;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

/// An inline-image protocol a terminal understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// iTerm2's OSC 1337 inline image, also implemented by `WezTerm`.
    Iterm2,
    /// The Kitty graphics protocol, also implemented by Ghostty and `WezTerm`.
    Kitty,
}

impl Protocol {
    /// Whether this protocol can carry a file of this type as it stands.
    #[must_use]
    pub fn carries(self, kind: Kind) -> bool {
        match self {
            // Direct transmission takes PNG; anything else would have to be
            // decoded to raw pixels first, which is a dependency this tool does
            // not need to carry for a convenience.
            Self::Kitty => kind == Kind::Png,
            Self::Iterm2 => true,
        }
    }
}

/// The image formats worth recognising, identified by their own bytes rather
/// than by a name somebody else chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Png,
    Jpeg,
    Gif,
    Webp,
}

impl Kind {
    #[must_use]
    pub fn of(bytes: &[u8]) -> Option<Self> {
        const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
        if bytes.starts_with(PNG) {
            return Some(Self::Png);
        }
        if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            return Some(Self::Jpeg);
        }
        if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            return Some(Self::Gif);
        }
        if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
            return Some(Self::Webp);
        }
        None
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
            Self::Gif => "GIF",
            Self::Webp => "WebP",
        }
    }
}

/// The protocol this terminal supports, if any.
///
/// Reads the environment rather than querying the terminal: a capability query
/// means putting the tty into raw mode and waiting for a reply that may never
/// come, on a tool whose whole argument is that it starts instantly.
#[must_use]
pub fn protocol() -> Option<Protocol> {
    if !std::io::stdout().is_terminal() {
        return None;
    }
    from_environment(&|name| std::env::var(name).ok())
}

/// The decision itself, over a lookup rather than the process environment, so
/// the rules can be tested without setting variables for every other test in
/// the binary.
fn from_environment(var: &dyn Fn(&str) -> Option<String>) -> Option<Protocol> {
    // Inside a multiplexer the terminal's own variables are inherited but its
    // graphics are not necessarily passed through.
    if var("TMUX").is_some() || var("STY").is_some() {
        return None;
    }

    let program = var("TERM_PROGRAM").unwrap_or_default();

    // Kitty and Ghostty each set a variable no other terminal does.
    if var("KITTY_WINDOW_ID").is_some() || var("GHOSTTY_RESOURCES_DIR").is_some() {
        return Some(Protocol::Kitty);
    }

    // WezTerm speaks both; its iTerm2 support covers more formats.
    if var("WEZTERM_PANE").is_some() || program == "WezTerm" {
        return Some(Protocol::Iterm2);
    }

    if program == "iTerm.app" {
        return Some(Protocol::Iterm2);
    }

    None
}

/// The escape sequence that draws these bytes, ready to write to stdout.
#[must_use]
pub fn draw(protocol: Protocol, bytes: &[u8], name: &str) -> String {
    let encoded = BASE64.encode(bytes);
    match protocol {
        Protocol::Iterm2 => {
            // `inline=1` draws it rather than offering it as a download; the
            // name is only a label, and the terminal never writes a file.
            let label = BASE64.encode(name.as_bytes());
            format!(
                "\x1b]1337;File=name={label};size={};inline=1;preserveAspectRatio=1:{encoded}\x07\n",
                bytes.len()
            )
        }
        Protocol::Kitty => {
            // Chunked, because the protocol caps one escape sequence at 4096
            // base64 characters.
            let mut out = String::with_capacity(encoded.len() + 256);
            let chunks: Vec<&str> = encoded
                .as_bytes()
                .chunks(4096)
                .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
                .collect();

            for (index, chunk) in chunks.iter().enumerate() {
                let more = u8::from(index + 1 < chunks.len());
                if index == 0 {
                    // f=100: the payload is a file in a format kitty decodes.
                    // a=T: transmit and display at once.
                    let _ = write!(out, "\x1b_Gf=100,a=T,m={more};{chunk}\x1b\\");
                } else {
                    let _ = write!(out, "\x1b_Gm={more};{chunk}\x1b\\");
                }
            }
            out.push('\n');
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name: &str| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        }
    }

    #[test]
    fn each_terminal_gets_the_protocol_it_implements() {
        assert_eq!(
            from_environment(&env(&[("KITTY_WINDOW_ID", "1")])),
            Some(Protocol::Kitty)
        );
        assert_eq!(
            from_environment(&env(&[("GHOSTTY_RESOURCES_DIR", "/x")])),
            Some(Protocol::Kitty)
        );
        assert_eq!(
            from_environment(&env(&[("TERM_PROGRAM", "iTerm.app")])),
            Some(Protocol::Iterm2)
        );
        assert_eq!(
            from_environment(&env(&[("WEZTERM_PANE", "0")])),
            Some(Protocol::Iterm2)
        );
    }

    /// The failure this guard exists for: escape codes printed as text.
    #[test]
    fn a_multiplexer_is_never_assumed_to_pass_graphics_through() {
        assert_eq!(
            from_environment(&env(&[("KITTY_WINDOW_ID", "1"), ("TMUX", "/tmp/s")])),
            None
        );
    }

    /// An unknown terminal is not a terminal that might work.
    #[test]
    fn anything_unrecognised_gets_nothing() {
        assert_eq!(from_environment(&env(&[("TERM", "xterm-256color")])), None);
        assert_eq!(from_environment(&env(&[])), None);
    }

    #[test]
    fn formats_are_read_from_the_bytes_not_the_name() {
        assert_eq!(Kind::of(b"\x89PNG\r\n\x1a\n\x00"), Some(Kind::Png));
        assert_eq!(Kind::of(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(Kind::Jpeg));
        assert_eq!(Kind::of(b"GIF89a..."), Some(Kind::Gif));
        assert_eq!(Kind::of(b"RIFF\0\0\0\0WEBPVP8 "), Some(Kind::Webp));
        assert_eq!(Kind::of(b"%PDF-1.7"), None);
        assert_eq!(Kind::of(b""), None);
    }

    /// Kitty's direct transmission decodes PNG and nothing else, so a JPEG has
    /// to fall back rather than be sent and silently dropped.
    #[test]
    fn kitty_takes_png_only() {
        assert!(Protocol::Kitty.carries(Kind::Png));
        assert!(!Protocol::Kitty.carries(Kind::Jpeg));
        assert!(Protocol::Iterm2.carries(Kind::Jpeg));
    }

    #[test]
    fn a_payload_over_the_chunk_limit_is_split_and_terminated() {
        let bytes = vec![0u8; 8000];
        let drawn = draw(Protocol::Kitty, &bytes, "big.png");

        assert!(drawn.starts_with("\x1b_Gf=100,a=T,m=1;"));
        assert!(drawn.contains("\x1b_Gm=0;"));
        assert!(drawn.ends_with("\x1b\\\n"));
    }

    #[test]
    fn the_iterm_sequence_carries_the_size_and_draws_inline() {
        let drawn = draw(Protocol::Iterm2, b"1234", "a.png");
        assert!(drawn.contains("size=4"));
        assert!(drawn.contains("inline=1"));
    }
}
