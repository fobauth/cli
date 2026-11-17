//! Answers laid out for a person: a list as a table of what matters, one thing as its name
//! and a few facts. Programs get the JSON instead.

use std::fmt::Write as _;

use serde_json::{Map, Value};

pub struct Style {
    pub color: bool,
    /// Seconds since 1970, for saying how long ago something was.
    pub now: i64,
}

impl Style {
    fn paint(&self, code: &str, text: &str) -> String {
        if self.color && !text.is_empty() {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }

    /// Anything that names what's beside or under it: a field, a section, a column.
    fn label(&self, text: &str) -> String {
        self.paint("38;5;248", text)
    }

    /// A value, coloured only where it says how something stands.
    fn value(&self, key: &str, text: &str) -> String {
        if !["status", "outcome", "state"].contains(&key) {
            return text.to_owned();
        }
        match text {
            "active" | "succeeded" | "enabled" | "verified" => self.paint("32", text),
            "invited" | "unverified" | "pending" | "next" | "locked" => self.paint("33", text),
            "retired" => self.paint("2", text),
            _ => self.paint("31", text),
        }
    }
}

/// Seconds since 1970 for an RFC 3339 time in UTC.
fn seconds(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    let shaped = bytes.len() >= 20
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && text.ends_with('Z');
    if !shaped {
        return None;
    }
    let part = |range: std::ops::Range<usize>| text[range].parse::<i64>().ok();
    let (year, month, day) = (part(0..4)?, part(5..7)?, part(8..10)?);
    let (year, month) = if month <= 2 {
        (year - 1, month + 9)
    } else {
        (year, month - 3)
    };
    let era = year.div_euclid(400);
    let of_era = year.rem_euclid(400);
    let of_year = (153 * month + 2) / 5 + day - 1;
    let days = era * 146_097 + of_era * 365 + of_era / 4 - of_era / 100 + of_year - 719_468;
    Some(days * 86_400 + part(11..13)? * 3600 + part(14..16)? * 60 + part(17..19)?)
}

fn ago(then: i64, now: i64) -> String {
    let passed = now - then;
    let count = |n: i64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    match passed {
        i64::MIN..0 => "in the future".to_owned(),
        0..60 => "just now".to_owned(),
        60..3600 => count(passed / 60, "minute"),
        3600..86_400 => count(passed / 3600, "hour"),
        86_400..2_592_000 => count(passed / 86_400, "day"),
        2_592_000..31_536_000 => count(passed / 2_592_000, "month"),
        _ => count(passed / 31_536_000, "year"),
    }
}

fn link(value: &Value) -> bool {
    value.as_str().is_some_and(|text| text.starts_with("http"))
}

/// Whether `text` has the shape of an ID rather than a name.
pub fn is_id(text: &str) -> bool {
    text.len() == 36 && text.bytes().filter(|byte| *byte == b'-').count() == 4
}

fn cell(value: &Value, style: &Style) -> String {
    let text = match value {
        Value::Null => String::new(),
        Value::Bool(true) => "yes".to_owned(),
        Value::Bool(false) => "no".to_owned(),
        Value::String(text) => match seconds(text) {
            Some(then) => ago(then, style.now),
            None => text.clone(),
        },
        Value::Array(items)
            if items
                .iter()
                .all(|item| !item.is_object() && !item.is_array()) =>
        {
            items
                .iter()
                .map(|item| cell(item, style))
                .collect::<Vec<_>>()
                .join(", ")
        }
        other => other.to_string(),
    };
    let text = text.replace('\n', " ");
    if text.chars().count() > 60 && !text.starts_with("http") {
        format!("{}…", text.chars().take(59).collect::<String>())
    } else {
        text
    }
}

/// The fields worth a column in the lists a command answers with.
const COLUMNS: &[(&str, &[&str])] = &[
    ("get_group", &["name", "email", "added_at"]),
    (
        "list_admins",
        &["name", "email", "role", "custom_role_name"],
    ),
    (
        "list_api_keys",
        &["name", "prefix", "scopes", "last_used_at", "expires_at"],
    ),
    ("list_apps", &["name", "kind", "access", "client_id"]),
    (
        "list_audit_log",
        &["created_at", "actor_email", "action", "ip"],
    ),
    (
        "list_connected_apps",
        &["name", "allowed_by", "last_used_at"],
    ),
    ("list_groups", &["name", "members", "description"]),
    (
        "list_identity_providers",
        &["name", "kind", "domains", "enabled"],
    ),
    ("list_networks", &["name", "cidrs"]),
    (
        "list_people",
        &["name", "email", "status", "last_sign_in_at"],
    ),
    (
        "list_person_activity",
        &["created_at", "actor_email", "action", "ip"],
    ),
    ("list_person_emails", &["email", "primary", "verified"]),
    (
        "list_sign_ins",
        &["created_at", "email", "outcome", "method", "ip", "location"],
    ),
    (
        "list_sign_on_policies",
        &["name", "rules", "apps", "is_default"],
    ),
    ("list_sign_on_rules", &["name", "action", "require_mfa"]),
    (
        "list_signing_keys",
        &["kid", "status", "algorithm", "created_at"],
    ),
    ("list_webhooks", &["url", "format", "enabled"]),
];

/// The columns for an operation's lists. Without any, a list shows what it has.
pub fn columns(operation: &str) -> &'static [&'static str] {
    COLUMNS
        .iter()
        .find(|(name, _)| *name == operation)
        .map(|(_, columns)| *columns)
        .unwrap_or_default()
}

/// What a thing goes by: the field people would call it.
const NAMES: &[&str] = &["name", "email", "domain", "url", "kid", "slug", "key"];

const ACRONYMS: &[&str] = &[
    "id", "ip", "url", "totp", "mfa", "saml", "acs", "kid", "pkce", "uri",
];

/// A field's name as a label: `last_sign_in_at` is `Last sign in`, `user_id` is `User ID`.
fn label(key: &str) -> String {
    let words: Vec<String> = key
        .strip_suffix("_at")
        .unwrap_or(key)
        .split('_')
        .enumerate()
        .map(|(i, word)| {
            if ACRONYMS.contains(&word) {
                word.to_uppercase()
            } else if i == 0 {
                let mut letters = word.chars();
                match letters.next() {
                    Some(first) => first.to_uppercase().chain(letters).collect(),
                    None => String::new(),
                }
            } else {
                word.to_owned()
            }
        })
        .collect();
    words.join(" ")
}

fn rows(value: &Value) -> Option<&Vec<Value>> {
    value
        .as_array()
        .filter(|items| !items.is_empty() && items.iter().all(Value::is_object))
}

/// The rows of an answer that is a list, or that wraps one in a field.
pub fn list(json: &Value) -> &[Value] {
    rows(json)
        .or_else(|| json.as_object()?.values().find_map(rows))
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// The columns a list shows: the ones asked for that it has, or failing that what it has.
fn chosen<'a>(rows: &'a [Value], wanted: &[&'a str]) -> Vec<&'a str> {
    let has = |column: &str| rows.iter().any(|row| !row[column].is_null());
    let mut columns: Vec<&str> = wanted
        .iter()
        .copied()
        .filter(|column| has(column))
        .collect();
    if columns.is_empty() {
        for row in rows {
            for (key, value) in row.as_object().into_iter().flatten() {
                if !value.is_object() && !value.is_null() && !columns.contains(&key.as_str()) {
                    columns.push(key);
                }
            }
        }
        columns.sort_by_key(|column| {
            NAMES
                .iter()
                .position(|name| name == column)
                .unwrap_or(NAMES.len())
        });
        columns.truncate(6);
    }
    columns.retain(|column| !rows.iter().any(|row| link(&row[*column])));
    // Beside a name an ID is noise. It stays in the details and in --json.
    if columns.iter().any(|column| NAMES.contains(column)) {
        columns.retain(|column| {
            !rows
                .iter()
                .any(|row| row[*column].as_str().is_some_and(is_id))
        });
    }
    columns
}

fn table(rows: &[Value], wanted: &[&str], style: &Style, indent: &str) -> String {
    let columns = chosen(rows, wanted);
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|column| cell(&row[*column], style))
                .collect()
        })
        .collect();
    let widths: Vec<usize> = columns
        .iter()
        .enumerate()
        .map(|(i, column)| {
            cells
                .iter()
                .map(|row| row[i].chars().count())
                .chain([label(column).len()])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |values: Vec<String>, paint: &dyn Fn(usize, &str) -> String| {
        let padded: Vec<String> = values
            .iter()
            .zip(&widths)
            .enumerate()
            .map(|(i, (value, width))| {
                let pad = " ".repeat(width - value.chars().count());
                format!("{}{pad}", paint(i, value))
            })
            .collect();
        format!("{indent}{}\n", padded.join("   ").trim_end())
    };
    let mut out = line(
        columns
            .iter()
            .map(|column| label(column).to_uppercase())
            .collect(),
        &|_, text| style.label(text),
    );
    for row in cells {
        out.push_str(&line(row, &|i, text| style.value(columns[i], text)));
    }
    out
}

/// One thing: what it goes by, its facts in a steady order, then a section for each thing
/// it holds. `counted` names the lists shown beside it, whose counts needn't be said twice.
fn thing(object: &Map<String, Value>, columns: &[&str], style: &Style, counted: &[&str]) -> String {
    let mut out = String::new();
    let mut facts: Vec<(&String, &Value)> = object
        .iter()
        .filter(|(key, value)| {
            !value.is_null()
                && !value.is_object()
                && rows(value).is_none()
                && !value.as_array().is_some_and(Vec::is_empty)
                && !(key.contains("picture") || key.contains("logo"))
                && !(value.is_number() && counted.contains(&key.as_str()))
        })
        .collect();
    facts.sort_by_key(|(key, _)| match key.as_str() {
        "name" => 0,
        "id" => 1,
        key if NAMES.contains(&key) || ["kind", "status", "role"].contains(&key) => 2,
        key if key.ends_with("_at") => 4,
        _ => 3,
    });
    let width = facts
        .iter()
        .map(|(key, _)| label(key).len())
        .max()
        .unwrap_or(0);
    for (key, value) in facts {
        let name = label(key);
        let pad = " ".repeat(width - name.len());
        let _ = writeln!(
            out,
            "{}{pad}  {}",
            style.label(&format!("{name}:")),
            style.value(key, &cell(value, style))
        );
    }
    out.push_str(&sections(object, columns, style));
    format!("{}\n", out.trim_end())
}

fn sections(object: &Map<String, Value>, columns: &[&str], style: &Style) -> String {
    let mut out = String::new();
    for (key, value) in object {
        let (heading, body) = match (rows(value), value.as_object()) {
            (Some(rows), _) => match chosen(rows, columns).as_slice() {
                // One column is a list of names, which reads better on the line.
                [only] => {
                    let names: Vec<String> =
                        rows.iter().map(|row| cell(&row[*only], style)).collect();
                    let _ = writeln!(
                        out,
                        "{}  {}",
                        style.label(&format!("{}:", label(key))),
                        names.join(", ")
                    );
                    continue;
                }
                _ => (label(key), table(rows, columns, style, "  ")),
            },
            (None, Some(inner)) => {
                let body: String = thing(inner, columns, style, &[])
                    .lines()
                    .map(|line| format!("  {line}\n"))
                    .collect();
                (label(key), body)
            }
            _ => continue,
        };
        if body.trim().is_empty() {
            continue;
        }
        let _ = write!(out, "{}\n{body}", style.label(&format!("{heading}:")));
    }
    out
}

pub fn readable(json: &Value, columns: &[&str], style: &Style) -> String {
    if json.as_array().is_some_and(Vec::is_empty) {
        return "Nothing to show.\n".to_owned();
    }
    if let Some(rows) = rows(json) {
        return table(rows, columns, style, "");
    }
    let Some(object) = json.as_object() else {
        return format!("{}\n", cell(json, style));
    };
    let holds = |value: &Value| rows(value).is_some() || value.is_object();
    let plain = object
        .values()
        .filter(|value| !value.is_null() && !holds(value))
        .count();
    let lists: Vec<&Vec<Value>> = object.values().filter_map(rows).collect();
    let inner = object.values().filter(|value| value.is_object()).count();
    // A page of a list, with nothing else worth a line.
    if let [list] = lists.as_slice()
        && inner == 0
        && plain <= 1
    {
        return table(list, columns, style, "");
    }
    // One thing wrapped with what it holds, such as a group and its members.
    let main = object.iter().find(|(_, value)| {
        value.as_object().is_some_and(|inner| {
            NAMES
                .iter()
                .any(|name| inner.get(*name).is_some_and(Value::is_string))
        })
    });
    if let Some((main_key, Value::Object(main))) = main
        && plain <= 2
    {
        let mut rest = object.clone();
        rest.remove(main_key);
        // A list of names alone sits with the fields, lined up with them.
        let mut main = main.clone();
        for (key, value) in object {
            if !value.is_null() && !holds(value) {
                main.entry(key.clone()).or_insert_with(|| value.clone());
                rest.remove(key);
            }
            if let Some(list) = rows(value)
                && let [only] = chosen(list, columns).as_slice()
            {
                let names: Vec<String> = list.iter().map(|row| cell(&row[*only], style)).collect();
                main.insert(key.clone(), Value::String(names.join(", ")));
                rest.remove(key);
            }
        }
        let counted: Vec<&str> = rest
            .iter()
            .filter(|(_, value)| rows(value).is_some())
            .map(|(key, _)| key.as_str())
            .collect();
        let mut out = thing(&main, columns, style, &counted);
        out.push_str(&sections(&rest, columns, style));
        return out;
    }
    thing(object, columns, style, &[])
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const NOW: i64 = 1_794_787_200;

    fn shown(json: Value, columns: &[&str]) -> Vec<String> {
        let style = Style {
            color: false,
            now: NOW,
        };
        readable(&json, columns, &style)
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn a_time_reads_as_how_long_ago() {
        assert_eq!(seconds("1970-01-02T00:00:00Z"), Some(86_400));
        assert_eq!(seconds("2026-11-16T00:00:00.5Z"), Some(NOW));
        assert_eq!(seconds("Engineering"), None);
        assert_eq!(ago(NOW - 90, NOW), "1 minute ago");
        assert_eq!(ago(NOW - 3 * 86_400, NOW), "3 days ago");
    }

    #[test]
    fn a_list_shows_what_was_asked_for_by_name() {
        let groups = json!([
            { "id": "01a1bff1-d059-735b-a417-7be6e535fcc7", "name": "Everyone", "members": 3, "built_in": "everyone" },
            { "id": "01a1c33e-57cd-7b9d-9452-5eac8f18a874", "name": "Engineering", "members": 2, "built_in": null },
        ]);
        assert_eq!(
            shown(groups, &["id", "name", "members"]),
            [
                "NAME          MEMBERS",
                "Everyone      3",
                "Engineering   2"
            ]
        );
        let page = json!({ "users": [{ "id": "1", "email": "a@b.test" }], "next": null });
        assert_eq!(shown(page, &[]), ["EMAIL      ID", "a@b.test   1"]);
        assert_eq!(shown(json!([]), &[]), ["Nothing to show."]);
    }

    #[test]
    fn finds_the_list_in_an_answer() {
        let page = json!({ "next": null, "people": [{ "id": "1" }, { "id": "2" }] });
        assert_eq!(list(&page).len(), 2);
        assert_eq!(list(&json!([{ "id": "1" }])).len(), 1);
        assert!(list(&json!({ "name": "Ada", "tags": ["a"] })).is_empty());
        assert!(is_id("01a1bff1-d059-735b-a417-7be6e535fcc7"));
        assert!(!is_id("Engineering"));
    }

    #[test]
    fn colour_marks_labels_and_how_things_stand() {
        let style = Style {
            color: true,
            now: NOW,
        };
        let person = json!({ "name": "Ada", "status": "disabled" });
        assert_eq!(
            readable(&person, &[], &style),
            "\x1b[38;5;248mName:\x1b[0m    Ada\n\x1b[38;5;248mStatus:\x1b[0m  \x1b[31mdisabled\x1b[0m\n"
        );
        let people = json!([{ "name": "Ada", "status": "active" }]);
        assert_eq!(
            readable(&people, &[], &style),
            "\x1b[38;5;248mNAME\x1b[0m   \x1b[38;5;248mSTATUS\x1b[0m\nAda    \x1b[32mactive\x1b[0m\n"
        );
    }

    #[test]
    fn one_thing_leads_with_its_name() {
        let group = json!({
            "group": { "id": "2", "name": "Engineering", "created_at": "2026-11-13T00:00:00Z", "description": null },
            "members": [{ "user_id": "9", "name": "Tom", "picture": "http://x.test/p.png" }],
        });
        assert_eq!(
            shown(group, &[]),
            [
                "Name:     Engineering",
                "ID:       2",
                "Created:  3 days ago",
                "Members:",
                "  NAME   USER ID",
                "  Tom    9",
            ]
        );
        let policy = json!({ "min_length": 12, "breach_check": true });
        assert_eq!(
            shown(policy, &[]),
            ["Breach check:  yes", "Min length:    12"]
        );
    }
}
