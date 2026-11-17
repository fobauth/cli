use std::fmt;

/// What went wrong, as a sentence for a person, a code for a program and the exit status.
#[derive(Debug)]
pub struct Error {
    pub message: String,
    pub code: String,
    pub exit: u8,
}

pub type Result<T> = std::result::Result<T, Error>;

pub const USAGE: u8 = 2;
pub const NOT_FOUND: u8 = 3;
pub const NOT_ALLOWED: u8 = 4;
pub const CONFLICT: u8 = 5;

impl Error {
    pub fn new(code: &str, exit: u8, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: code.to_owned(),
            exit,
        }
    }

    /// The command was typed wrong.
    pub fn usage(message: impl Into<String>) -> Self {
        Self::new("usage", USAGE, message)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<String> for Error {
    fn from(message: String) -> Self {
        Self::new("error", 1, message)
    }
}

impl From<&str> for Error {
    fn from(message: &str) -> Self {
        Self::new("error", 1, message)
    }
}

impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        let host = error
            .url()
            .and_then(|url| url.host_str())
            .unwrap_or("the server");
        let message = if error.is_timeout() {
            format!("{host} took too long to answer.")
        } else if error.is_connect() || error.is_request() {
            format!("Can't reach {host}.")
        } else {
            format!("Couldn't read the answer from {host}.")
        };
        Self::new("unreachable", 1, message)
    }
}

pub fn file(path: &std::path::Path, error: std::io::Error) -> Error {
    Error::new("file", 1, format!("{}: {error}", path.display()))
}

/// The closest of `known` to what was typed, when it's close enough to be a slip.
pub fn closest<'a>(typed: &str, known: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    known
        .into_iter()
        .map(|name| (distance(typed, name), name))
        .filter(|(distance, name)| *distance <= 2 || name.starts_with(typed))
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, name)| name)
}

fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, from) in a.iter().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, to) in b.iter().enumerate() {
            let cost = previous + usize::from(from != to);
            previous = row[j + 1];
            row[j + 1] = cost.min(previous + 1).min(row[j] + 1);
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slip_is_matched_to_what_was_meant() {
        let known = ["people", "groups", "apps"];
        assert_eq!(closest("poeple", known), Some("people"));
        assert_eq!(closest("group", known), Some("groups"));
        assert_eq!(closest("webhooks", known), None);
    }
}
