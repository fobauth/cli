use std::{collections::BTreeMap, path::PathBuf};

use clap::Args;
use serde_json::Value;

use crate::{
    catalogue::{self, Operation},
    error::{Error, Result, closest},
};

#[derive(Args, Debug, Default, Clone, PartialEq)]
pub struct Global {
    #[arg(long, env = "FOB_ORG", global = true, value_name = "SLUG")]
    pub org: Option<String>,
    #[arg(
        long,
        env = "FOB_API_KEY",
        global = true,
        value_name = "KEY",
        hide_env_values = true
    )]
    pub api_key: Option<String>,
    #[arg(long, env = "FOB_HOST", global = true, value_name = "DOMAIN")]
    pub host: Option<String>,
    #[arg(long, env = "FOB_CONSOLE", global = true, value_name = "URL")]
    pub console: Option<String>,
    #[arg(long, global = true)]
    pub json: bool,
    #[arg(long, global = true)]
    pub text: bool,
    #[arg(long, global = true)]
    pub refresh: bool,
}

#[derive(Debug, Default, PartialEq)]
pub struct RunArgs {
    pub params: BTreeMap<String, String>,
    pub query: Vec<(String, String)>,
    pub data: Option<String>,
    pub fields: Vec<(String, Value)>,
    pub file: Option<PathBuf>,
    pub output: Option<PathBuf>,
    pub yes: bool,
    pub help: bool,
}

/// A value typed for a field, as the JSON the field's type calls for.
fn typed(kind: &str, raw: &str) -> Value {
    let loose = || match serde_json::from_str(raw) {
        Ok(value @ (Value::Number(_) | Value::Bool(_) | Value::Null)) => value,
        _ => Value::String(raw.to_owned()),
    };
    if let Some(item) = kind.strip_suffix("[]") {
        return match serde_json::from_str(raw) {
            Ok(list @ Value::Array(_)) => list,
            _ => Value::Array(
                raw.split(',')
                    .filter(|part| !part.is_empty())
                    .map(|part| typed(item, part.trim()))
                    .collect(),
            ),
        };
    }
    let kinds: Vec<&str> = kind.split('|').collect();
    if kinds
        .iter()
        .any(|kind| matches!(*kind, "integer" | "number" | "boolean"))
    {
        loose()
    } else if kinds.contains(&"object") {
        serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_owned()))
    } else if raw == "null" && kinds.contains(&"null") {
        Value::Null
    } else {
        Value::String(raw.to_owned())
    }
}

/// Reads what follows a command: the IDs its path takes, in order, and a flag for each
/// field of its body and query. Global flags are accepted here too.
pub fn parse(operation: &Operation, args: &[String], global: &mut Global) -> Result<RunArgs> {
    let mut run = RunArgs::default();
    let params = operation.params();
    let body = operation.body.as_ref().map(catalogue::fields);
    let query = operation.query.as_ref().map(catalogue::fields);
    let field = |fields: &Option<Vec<catalogue::Field>>, name: &str| {
        fields
            .iter()
            .flatten()
            .find(|field| field.name == name)
            .map(|field| field.kind.clone())
    };
    let mut rest = args.iter().peekable();
    while let Some(arg) = rest.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_owned())),
            _ => (arg.as_str(), None),
        };
        if !flag.starts_with('-') || flag == "-" {
            let Some(name) = params.iter().find(|name| !run.params.contains_key(**name)) else {
                return Err(Error::usage(format!(
                    "Unexpected {arg}. Flags come as --name value."
                )));
            };
            run.params.insert((*name).to_owned(), arg.clone());
            continue;
        }
        match flag {
            "--json" => global.json = true,
            "--text" => global.text = true,
            "--refresh" => global.refresh = true,
            "--yes" | "-y" => run.yes = true,
            "--help" | "-h" => run.help = true,
            _ => {
                let name = flag.trim_start_matches('-').replace('-', "_");
                let kind = field(&body, &name).or_else(|| field(&query, &name));
                let switch = kind.as_deref().is_some_and(|kind| kind.contains("boolean"))
                    && inline.is_none()
                    && rest.peek().is_none_or(|next| next.starts_with("--"));
                let value = if switch {
                    "true".to_owned()
                } else {
                    match inline.or_else(|| rest.next().cloned()) {
                        Some(value) => value,
                        None => return Err(Error::usage(format!("{flag} needs a value."))),
                    }
                };
                match flag {
                    "--org" => global.org = Some(value),
                    "--api-key" => global.api_key = Some(value),
                    "--host" => global.host = Some(value),
                    "--console" => global.console = Some(value),
                    "-d" | "--data" => run.data = Some(value),
                    "--file" => run.file = Some(value.into()),
                    "-o" | "--output" => run.output = Some(value.into()),
                    _ => match (field(&body, &name), field(&query, &name)) {
                        (Some(kind), _) => {
                            let value = typed(&kind, &value);
                            match run.fields.iter_mut().find(|(key, _)| *key == name) {
                                Some((_, Value::Array(list))) if kind.ends_with("[]") => {
                                    list.extend(value.as_array().cloned().unwrap_or_default());
                                }
                                Some((_, existing)) => *existing = value,
                                None => run.fields.push((name, value)),
                            }
                        }
                        (None, Some(_)) => run.query.push((name, value)),
                        (None, None) => {
                            let known: Vec<String> = body
                                .iter()
                                .chain(&query)
                                .flatten()
                                .map(|field| field.name.replace('_', "-"))
                                .collect();
                            let dashed = name.replace('_', "-");
                            let hint = closest(&dashed, known.iter().map(String::as_str))
                                .map(|meant| format!(" Did you mean --{meant}?"))
                                .unwrap_or_default();
                            return Err(Error::usage(format!(
                                "Unknown flag {flag}.{hint} Add --help to see the flags."
                            )));
                        }
                    },
                }
            }
        }
    }
    if run.help {
        return Ok(run);
    }
    if run.data.is_some() && !run.fields.is_empty() {
        return Err(Error::usage("Use -d or field flags, not both."));
    }
    if let Some(missing) = params.iter().find(|name| !run.params.contains_key(**name)) {
        return Err(Error::usage(format!(
            "Missing <{missing}>. Add --help to see what the command takes."
        )));
    }
    Ok(run)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn operation() -> Operation {
        serde_json::from_value(json!({
            "name": "update_thing",
            "group": "things",
            "summary": "",
            "method": "PATCH",
            "path": "/orgs/{org}/things/{thing}/parts/{part}",
            "query": { "type": "object", "properties": { "limit": { "type": "integer" } } },
            "body": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "count": { "type": "integer" },
                    "enabled": { "type": "boolean" },
                    "redirect_uris": { "type": "array", "items": { "type": "string" } },
                },
            },
        }))
        .unwrap()
    }

    fn parsed(args: &[&str]) -> Result<(RunArgs, Global)> {
        let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        let mut global = Global::default();
        parse(&operation(), &args, &mut global).map(|run| (run, global))
    }

    #[test]
    fn ids_come_in_order_and_fields_as_flags() {
        let (run, global) = parsed(&[
            "t1",
            "p2",
            "--name",
            "12",
            "--count=3",
            "--enabled",
            "--redirect-uris",
            "https://a.test,https://b.test",
            "--redirect-uris",
            "https://c.test",
            "--limit",
            "5",
            "--org",
            "acme",
            "--yes",
        ])
        .unwrap();
        assert_eq!(run.params["thing"], "t1");
        assert_eq!(run.params["part"], "p2");
        assert_eq!(
            run.fields,
            [
                ("name".to_owned(), json!("12")),
                ("count".to_owned(), json!(3)),
                ("enabled".to_owned(), json!(true)),
                (
                    "redirect_uris".to_owned(),
                    json!(["https://a.test", "https://b.test", "https://c.test"])
                ),
            ]
        );
        assert_eq!(run.query, [("limit".to_owned(), "5".to_owned())]);
        assert_eq!(global.org.as_deref(), Some("acme"));
        assert!(run.yes);
    }

    #[test]
    fn slips_are_usage_errors_that_say_what_was_meant() {
        let unknown = parsed(&["t1", "p2", "--nmae", "x"]).unwrap_err();
        assert_eq!(unknown.exit, crate::error::USAGE);
        assert!(unknown.message.contains("--name"), "{}", unknown.message);
        assert!(parsed(&["a", "b", "c"]).is_err());
        assert!(parsed(&["t1", "p2", "--name"]).is_err());
        assert!(parsed(&["t1", "p2", "-d", "{}", "--name", "x"]).is_err());
        assert_eq!(
            parsed(&["t1", "p2", "--enabled", "false"])
                .unwrap()
                .0
                .fields[0]
                .1,
            json!(false)
        );
    }

    #[test]
    fn a_missing_id_is_named_unless_help_was_asked_for() {
        let missing = parsed(&["t1", "--name", "x"]).unwrap_err();
        assert_eq!(missing.exit, crate::error::USAGE);
        assert!(missing.message.contains("<part>"), "{}", missing.message);
        assert!(parsed(&["--help"]).unwrap().0.help);
    }

    #[test]
    fn text_and_json_are_read_after_the_command() {
        let (_, global) = parsed(&["t1", "p2", "--text", "--json"]).unwrap();
        assert!(global.text && global.json);
    }
}
