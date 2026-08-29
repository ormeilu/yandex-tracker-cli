//! Durations, in the two forms this tool has to speak.
//!
//! Tracker takes and returns ISO 8601 durations — `PT1H30M`. Nobody types that,
//! and nobody wants to read it either, so `1h30m` goes in and `1h 30m` comes
//! out. The ISO form is still accepted on input: a caller who already has one,
//! or who is scripting against the API's own vocabulary, should not have to
//! translate it into ours and back.

/// Parse `1h30m`, `45m`, `2d`, `1w`, or an ISO 8601 duration, into ISO 8601.
///
/// Weeks and days are what people log time in; Tracker counts a working day as
/// 8 hours and a working week as 5 days, and it does that conversion itself, so
/// `P1D` is passed through as a day rather than turned into 24 hours here.
///
/// # Errors
///
/// Returns a message naming what was not understood.
pub fn to_iso8601(input: &str) -> Result<String, String> {
    use std::fmt::Write as _;

    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("empty duration".to_owned());
    }

    // Already ISO: hand it back untouched rather than parsing and re-emitting,
    // which could only lose information.
    let upper = trimmed.to_uppercase();
    if upper.starts_with('P') {
        return Ok(upper);
    }

    let mut date = String::new();
    let mut time = String::new();
    let mut number = String::new();
    let mut seen = false;

    for character in trimmed.chars() {
        if character.is_ascii_digit() {
            number.push(character);
            continue;
        }
        if character.is_whitespace() {
            continue;
        }
        if number.is_empty() {
            return Err(format!("`{input}`: `{character}` has no number before it"));
        }

        let unit = character.to_ascii_lowercase();
        match unit {
            'w' | 'd' => {
                let _ = write!(date, "{number}{}", unit.to_ascii_uppercase());
            }
            'h' | 'm' | 's' => {
                let _ = write!(time, "{number}{}", unit.to_ascii_uppercase());
            }
            other => {
                return Err(format!(
                    "`{input}`: unknown unit `{other}` (use w, d, h, m or s)"
                ));
            }
        }
        number.clear();
        seen = true;
    }

    if !number.is_empty() {
        return Err(format!(
            "`{input}`: `{number}` has no unit (w, d, h, m or s)"
        ));
    }
    if !seen {
        return Err(format!("`{input}`: no duration in it"));
    }

    if time.is_empty() {
        Ok(format!("P{date}"))
    } else {
        Ok(format!("P{date}T{time}"))
    }
}

/// Whole minutes as the ISO 8601 duration Tracker takes.
///
/// Rounded to the minute, because that is the resolution a worklog is read at,
/// and never to zero: a timer that ran for forty seconds recorded nothing at
/// all would be a surprise, and Tracker has no use for `PT0M`.
#[must_use]
pub fn from_minutes(minutes: i64) -> String {
    let minutes = minutes.max(1);
    let (hours, rest) = (minutes / 60, minutes % 60);
    match (hours, rest) {
        (0, minutes) => format!("PT{minutes}M"),
        (hours, 0) => format!("PT{hours}H"),
        (hours, minutes) => format!("PT{hours}H{minutes}M"),
    }
}

/// Render an ISO 8601 duration the way it was typed: `PT1H30M` becomes `1h 30m`.
///
/// Anything unrecognised is returned as it came. A duration we cannot read is
/// still a fact about the worklog, and replacing it with a dash would hide it.
#[must_use]
pub fn human(iso: &str) -> String {
    let Some(rest) = iso.strip_prefix('P') else {
        return iso.to_owned();
    };

    let mut out = Vec::new();
    let mut number = String::new();
    for character in rest.chars() {
        match character {
            'T' => {}
            digit if digit.is_ascii_digit() => number.push(digit),
            unit if !number.is_empty() => {
                out.push(format!("{number}{}", unit.to_ascii_lowercase()));
                number.clear();
            }
            _ => return iso.to_owned(),
        }
    }

    if out.is_empty() {
        iso.to_owned()
    } else {
        out.join(" ")
    }
}

#[cfg(test)]
// An `unwrap_err` in a test is the test asserting; a wrong value fails it.
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn what_people_type_becomes_what_tracker_takes() {
        assert_eq!(to_iso8601("1h30m"), Ok("PT1H30M".to_owned()));
        assert_eq!(to_iso8601("45m"), Ok("PT45M".to_owned()));
        assert_eq!(to_iso8601("2h"), Ok("PT2H".to_owned()));
        assert_eq!(to_iso8601("1d"), Ok("P1D".to_owned()));
        assert_eq!(to_iso8601("1w2d"), Ok("P1W2D".to_owned()));
        assert_eq!(to_iso8601("1d 4h"), Ok("P1DT4H".to_owned()));
    }

    /// A caller who already has an ISO duration should not have to translate it
    /// into our shorthand for us to translate it straight back.
    #[test]
    fn an_iso_duration_passes_through() {
        assert_eq!(to_iso8601("PT1H30M"), Ok("PT1H30M".to_owned()));
        assert_eq!(to_iso8601("pt30m"), Ok("PT30M".to_owned()));
    }

    /// The message has to say what was wrong with what they typed. "invalid
    /// duration" sends someone to the documentation for a missing letter.
    #[test]
    fn a_bad_duration_says_what_is_wrong_with_it() {
        assert!(to_iso8601("90").unwrap_err().contains("no unit"));
        assert!(to_iso8601("h").unwrap_err().contains("no number"));
        assert!(to_iso8601("1y").unwrap_err().contains("unknown unit"));
        assert!(to_iso8601("").is_err());
    }

    #[test]
    fn iso_durations_are_read_back_as_they_were_typed() {
        assert_eq!(human("PT1H30M"), "1h 30m");
        assert_eq!(human("P1DT4H"), "1d 4h");
        assert_eq!(human("PT45M"), "45m");
    }

    /// A duration we cannot read is still a fact about the worklog.
    #[test]
    fn something_unreadable_is_passed_through_not_hidden() {
        assert_eq!(human("nonsense"), "nonsense");
        assert_eq!(human("P"), "P");
    }
}

#[cfg(test)]
mod minutes {
    use super::*;

    #[test]
    fn minutes_become_hours_and_minutes() {
        assert_eq!(from_minutes(90), "PT1H30M");
        assert_eq!(from_minutes(45), "PT45M");
        assert_eq!(from_minutes(120), "PT2H");
    }

    /// A short timer records something rather than nothing.
    #[test]
    fn nothing_at_all_is_still_a_minute() {
        assert_eq!(from_minutes(0), "PT1M");
        assert_eq!(from_minutes(-5), "PT1M");
    }
}
