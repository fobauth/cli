use std::{
    collections::BTreeMap,
    fmt::Write,
    fs,
    path::PathBuf,
    time::{Duration, SystemTime},
};

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result, closest, file};

const FRESH_FOR: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub name: String,
    pub group: String,
    pub summary: String,
    pub method: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
    #[serde(default)]
    pub upload: bool,
    #[serde(default)]
    pub download: bool,
}

#[derive(Debug, PartialEq)]
pub struct Field {
    pub name: String,
    pub kind: String,
    pub required: bool,
    pub description: String,
}

impl Operation {
    /// The path's parameters other than `org`, which a command takes as plain arguments
    /// in this order.
    pub fn params(&self) -> Vec<&str> {
        self.path
            .split('/')
            .filter_map(placeholder)
            .filter(|name| *name != "org")
            .collect()
    }

    pub fn fill(&self, org: &str, params: &BTreeMap<String, String>) -> Result<String> {
        let parts = self
            .path
            .split('/')
            .map(|part| {
                let value = match placeholder(part) {
                    None => return Ok(part.to_owned()),
                    Some("org") => org,
                    Some(name) => params
                        .get(name)
                        .ok_or_else(|| Error::usage(format!("Missing <{name}>.")))?,
                };
                Ok(encode(value))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(parts.join("/"))
    }

    /// What `--help` shows for the command typed as `typed`.
    pub fn usage(&self, typed: &str) -> String {
        let ids: String = self
            .params()
            .iter()
            .map(|name| format!(" <{name}>"))
            .collect();
        let body = self.body.as_ref().map(fields).unwrap_or_default();
        let query = self.query.as_ref().map(fields).unwrap_or_default();
        let mut rows: Vec<(String, String)> = Vec::new();
        for field in body.iter().chain(&query) {
            let mut about = field.description.clone();
            if field.required {
                about = format!("Required. {about}").trim_end().to_owned();
            }
            rows.push((
                format!("--{} <{}>", field.name.replace('_', "-"), field.kind),
                about,
            ));
        }
        if self.body.is_some() {
            rows.push(("-d <json|@file|->".into(), "The whole body as JSON".into()));
        }
        if self.upload {
            rows.push(("--file <path>".into(), "Required. The file to send".into()));
        }
        if self.download {
            rows.push(("-o <path>".into(), "Where to save the file".into()));
        }
        if self.method == "DELETE" {
            rows.push(("--yes".into(), "Don't ask first".into()));
        }
        rows.push(("--json".into(), "Print JSON".into()));

        let mut example = format!("fob {typed}{ids}");
        for field in body.iter().filter(|field| field.required) {
            let _ = write!(example, " --{0} <{0}>", field.name.replace('_', "-"));
        }
        if self.upload {
            example.push_str(" --file <path>");
        }

        let mut out = format!(
            "{}\n\nUsage: fob {typed}{ids} [flags]\n\nFlags:\n",
            self.summary
        );
        let width = rows.iter().map(|(flag, _)| flag.len()).max().unwrap_or(0);
        for (flag, about) in rows {
            let _ = writeln!(out, "{}", format!("  {flag:width$}  {about}").trim_end());
        }
        let _ = write!(out, "\nExample:\n  {example}\n");
        out
    }
}

fn placeholder(part: &str) -> Option<&str> {
    part.strip_prefix('{')?.strip_suffix('}')
}

fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

fn singular(word: &str) -> String {
    match word {
        "people" => "person".to_owned(),
        _ => match word.strip_suffix("ies") {
            Some(stem) => format!("{stem}y"),
            None => word.strip_suffix('s').unwrap_or(word).to_owned(),
        },
    }
}

/// An operation's name without the words its group already says: `create_person` in
/// `people` is `create`.
fn short(operation: &Operation) -> String {
    let mut drop: Vec<String> = operation.group.split('-').map(str::to_owned).collect();
    drop.extend(operation.group.split('-').map(singular));
    if operation.group == "policies" {
        drop.extend(["sign".to_owned(), "on".to_owned()]);
    }
    let kept: Vec<&str> = operation
        .name
        .split('_')
        .filter(|word| !drop.iter().any(|dropped| dropped == word))
        .collect();
    kept.join("-")
}

/// What to type after the group to run an operation. The short form, unless two in the
/// group would share it.
pub fn command(operations: &[Operation], operation: &Operation) -> String {
    let name = short(operation);
    let shared = operations
        .iter()
        .filter(|other| other.group == operation.group && short(other) == name)
        .count();
    if name.is_empty() || shared > 1 {
        operation.name.replace('_', "-")
    } else {
        name
    }
}

/// The groups, in the order the server first lists them.
pub fn groups(operations: &[Operation]) -> Vec<&str> {
    let mut groups: Vec<&str> = Vec::new();
    for operation in operations {
        if !groups.contains(&operation.group.as_str()) {
            groups.push(&operation.group);
        }
    }
    groups
}

pub fn in_group<'a>(operations: &'a [Operation], group: &str) -> Vec<&'a Operation> {
    operations
        .iter()
        .filter(|operation| operation.group == group)
        .collect()
}

/// The operation `words` start with, and how many of them name it: a group then one of
/// its commands, or an operation's full name.
pub fn from_words<'a>(
    operations: &'a [Operation],
    words: &[String],
) -> Result<(&'a Operation, usize)> {
    let first = words.first().map(String::as_str).unwrap_or_default();
    if let Some(second) = words.get(1)
        && !second.starts_with('-')
        && operations.iter().any(|operation| operation.group == first)
    {
        let wanted = second.replace('_', "-");
        return in_group(operations, first)
            .into_iter()
            .find(|operation| command(operations, operation) == wanted)
            .map(|operation| (operation, 2))
            .ok_or_else(|| {
                let commands: Vec<String> = in_group(operations, first)
                    .into_iter()
                    .map(|operation| command(operations, operation))
                    .collect();
                let hint = closest(&wanted, commands.iter().map(String::as_str))
                    .map(|meant| format!(" Did you mean {meant}?"))
                    .unwrap_or_default();
                Error::usage(format!(
                    "No {first} command {second}.{hint} Run `fob {first}` to list them."
                ))
            });
    }
    find(operations, first).map(|operation| (operation, 1))
}

/// The operations whose group, name or summary has every one of `words`.
pub fn search<'a>(operations: &'a [Operation], words: &[String]) -> Vec<&'a Operation> {
    let words: Vec<String> = words.iter().map(|word| word.to_lowercase()).collect();
    operations
        .iter()
        .filter(|operation| {
            let text = format!(
                "{} {} {}",
                operation.group, operation.name, operation.summary
            )
            .to_lowercase();
            words.iter().all(|word| text.contains(word))
        })
        .collect()
}

pub fn find<'a>(operations: &'a [Operation], name: &str) -> Result<&'a Operation> {
    let wanted = name.replace('-', "_");
    operations
        .iter()
        .find(|operation| operation.name == wanted)
        .ok_or_else(|| {
            let mut known = groups(operations);
            known.extend(["login", "logout", "whoami", "search", "help"]);
            let hint = closest(name, known)
                .map(|meant| format!(" Did you mean {meant}?"))
                .unwrap_or_default();
            Error::usage(format!("No command {name}.{hint} Run `fob` to list them."))
        })
}

pub fn fields(schema: &Value) -> Vec<Field> {
    let required: Vec<&str> = schema["required"]
        .as_array()
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let Some(properties) = schema["properties"].as_object() else {
        return Vec::new();
    };
    let mut fields: Vec<Field> = properties
        .iter()
        .map(|(name, property)| Field {
            name: name.clone(),
            kind: kind(schema, property),
            required: required.contains(&name.as_str()),
            description: property["description"]
                .as_str()
                .or(resolve(schema, property)["description"].as_str())
                .unwrap_or_default()
                .replace('\n', " "),
        })
        .collect();
    fields.sort_by_key(|field| !field.required);
    fields
}

fn resolve<'a>(root: &'a Value, schema: &'a Value) -> &'a Value {
    schema["$ref"]
        .as_str()
        .and_then(|reference| reference.strip_prefix("#/$defs/"))
        .map(|name| &root["$defs"][name])
        .unwrap_or(schema)
}

fn kind(root: &Value, schema: &Value) -> String {
    let schema = resolve(root, schema);
    if let Some(value) = schema.get("const") {
        return plain(value);
    }
    if let Some(values) = schema["enum"].as_array() {
        return values.iter().map(plain).collect::<Vec<_>>().join("|");
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(options) = schema[key].as_array() {
            return join(options.iter().map(|option| kind(root, option)));
        }
    }
    match &schema["type"] {
        Value::String(name) if name == "array" => format!("{}[]", kind(root, &schema["items"])),
        Value::String(name) => name.clone(),
        Value::Array(names) => join(names.iter().map(plain)),
        _ => "any".to_owned(),
    }
}

fn join(kinds: impl Iterator<Item = String>) -> String {
    let mut kinds: Vec<String> = kinds.filter(|kind| kind != "null").collect();
    kinds.dedup();
    if kinds.is_empty() {
        "null".to_owned()
    } else {
        kinds.join("|")
    }
}

fn plain(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

pub fn load(client: &Client, console: &str, refresh: bool) -> Result<Vec<Operation>> {
    let cache = cache_file(console);
    if !refresh
        && let Some(path) = &cache
        && let Ok(modified) = fs::metadata(path).and_then(|meta| meta.modified())
        && SystemTime::now()
            .duration_since(modified)
            .unwrap_or(FRESH_FOR)
            < FRESH_FOR
        && let Ok(text) = fs::read_to_string(path)
        && let Ok(operations) = serde_json::from_str(&text)
    {
        return Ok(operations);
    }
    let response = client.get(format!("{console}/api/operations")).send()?;
    if !response.status().is_success() {
        return Err(format!(
            "{console} didn't list its commands ({}). Check --host.",
            response.status()
        )
        .into());
    }
    let operations: Vec<Operation> = response.json()?;
    if let Some(path) = cache {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|error| file(dir, error))?;
        }
        let text = serde_json::to_string(&operations).expect("operations serialize");
        fs::write(&path, text).map_err(|error| file(&path, error))?;
    }
    Ok(operations)
}

fn cache_file(console: &str) -> Option<PathBuf> {
    let name: String = console
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    Some(
        crate::store::dirs()?
            .cache_dir()
            .join(format!("operations-{name}.json")),
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn operation(name: &str, summary: &str, path: &str) -> Operation {
        Operation {
            name: name.into(),
            group: "people".into(),
            summary: summary.into(),
            method: "GET".into(),
            path: path.into(),
            query: None,
            body: None,
            upload: false,
            download: false,
        }
    }

    fn params(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn fills_paths() {
        let op = operation(
            "remove_group_member",
            "",
            "/orgs/{org}/groups/{group}/members/{user}",
        );
        assert_eq!(op.params(), ["group", "user"]);
        assert_eq!(
            op.fill(
                "clyent",
                &params(&[("group", "g 1"), ("user", "a/b@c.test")])
            )
            .unwrap(),
            "/orgs/clyent/groups/g%201/members/a%2Fb%40c.test"
        );
    }

    #[test]
    fn a_missing_param_is_refused() {
        let op = operation("get_group", "", "/orgs/{org}/groups/{group}");
        let missing = op.fill("clyent", &params(&[])).unwrap_err();
        assert_eq!(missing.message, "Missing <group>.");
    }

    #[test]
    fn filters_by_every_word() {
        let ops = [
            operation("list_groups", "List the groups.", "/orgs/{org}/groups"),
            operation("create_group", "Create a group.", "/orgs/{org}/groups"),
            operation(
                "list_people",
                "Search the people in the directory.",
                "/orgs/{org}/users",
            ),
        ];
        let names = |words: &[&str]| -> Vec<&str> {
            let words: Vec<String> = words.iter().map(|word| word.to_string()).collect();
            search(&ops, &words)
                .iter()
                .map(|op| op.name.as_str())
                .collect()
        };
        assert_eq!(names(&[]).len(), 3);
        assert_eq!(names(&["group"]), ["list_groups", "create_group"]);
        assert_eq!(names(&["LIST", "group"]), ["list_groups"]);
        assert_eq!(names(&["directory"]), ["list_people"]);
        assert!(names(&["webhook"]).is_empty());
    }

    #[test]
    fn finds_by_name_with_dashes_or_underscores() {
        let ops = [operation("list_groups", "", "/orgs/{org}/groups")];
        assert!(find(&ops, "list-groups").is_ok());
        assert!(find(&ops, "list_group").is_err());
    }

    #[test]
    fn describes_schema_fields() {
        let schema = json!({
            "$defs": { "AppKind": { "oneOf": [
                { "const": "web", "type": "string" },
                { "const": "spa", "type": "string" }
            ] } },
            "properties": {
                "acs_url": { "description": "Where to send\nresponses.", "type": ["string", "null"] },
                "kind": { "$ref": "#/$defs/AppKind" },
                "limit": { "format": "int64", "type": ["integer", "null"] },
                "name": { "type": "string" },
                "redirect_uris": { "items": { "type": "string" }, "type": "array" },
                "role": { "enum": ["owner", "admin"], "type": "string" }
            },
            "required": ["name", "kind"],
            "type": "object"
        });
        let fields = fields(&schema);
        let rows: Vec<(&str, &str, bool)> = fields
            .iter()
            .map(|field| (field.name.as_str(), field.kind.as_str(), field.required))
            .collect();
        assert_eq!(
            rows,
            [
                ("kind", "web|spa", true),
                ("name", "string", true),
                ("acs_url", "string", false),
                ("limit", "integer", false),
                ("redirect_uris", "string[]", false),
                ("role", "owner|admin", false),
            ]
        );
        assert_eq!(fields[2].description, "Where to send responses.");
    }

    #[test]
    fn describes_an_operation() {
        let mut op = operation("get_group", "Get a group.", "/orgs/{org}/groups/{group}");
        op.query = Some(json!({ "properties": { "after": { "type": ["string", "null"] } } }));
        let text = op.usage("groups get");
        assert!(text.contains("Usage: fob groups get <group> [flags]"));
        assert!(text.contains("--after <string>"));
        assert!(text.contains("Example:\n  fob groups get <group>"));
    }

    #[test]
    fn a_command_is_the_name_without_what_the_group_says() {
        let all = [
            operation("list_people", "", ""),
            operation("create_person", "", ""),
            operation("list_person_emails", "", ""),
            operation("add_person_to_group", "", ""),
        ];
        let commands: Vec<String> = all.iter().map(|op| command(&all, op)).collect();
        assert_eq!(commands, ["list", "create", "list-emails", "add-to-group"]);

        let words = [
            "people".to_owned(),
            "create".to_owned(),
            "--name".to_owned(),
        ];
        let (found, used) = from_words(&all, &words).unwrap();
        assert_eq!((found.name.as_str(), used), ("create_person", 2));
        let full = ["list-people".to_owned()];
        assert_eq!(from_words(&all, &full).unwrap().1, 1);
        assert_eq!(groups(&all), ["people"]);
    }

    #[test]
    fn commands_that_would_clash_keep_their_full_names() {
        let all = [
            operation("get_person", "", ""),
            operation("get_people", "", ""),
        ];
        assert_eq!(command(&all, &all[0]), "get-person");
    }
}
