use std::{
    fs,
    io::{self, IsTerminal, Read, Write},
    time::{SystemTime, UNIX_EPOCH},
};

use reqwest::{Method, StatusCode, blocking::Client, header};
use serde_json::{Map, Value};

use crate::{
    args::{Global, RunArgs},
    catalogue::Operation,
    error::{CONFLICT, Error, NOT_ALLOWED, NOT_FOUND, Result, USAGE, file},
    show,
};

/// Where a request goes and who it is from.
pub struct Api<'a> {
    pub client: &'a Client,
    pub console: &'a str,
    pub org: &'a str,
    pub token: &'a str,
    /// What to do when the server doesn't accept the token.
    pub if_unauthorized: &'a str,
}

pub enum Payload {
    None,
    Json(Value),
    File(Vec<u8>),
}

pub struct Answer {
    pub bytes: Vec<u8>,
    pub json: Option<Value>,
}

/// What a command sends, read and checked before anything is asked or sent.
pub fn payload(operation: &Operation, args: &RunArgs) -> Result<Payload> {
    let body = body(operation, args)?;
    if operation.upload {
        let path = args
            .file
            .as_ref()
            .ok_or_else(|| Error::usage("Missing --file <path>."))?;
        return fs::read(path)
            .map(Payload::File)
            .map_err(|error| file(path, error));
    }
    if args.file.is_some() {
        return Err(Error::usage("This command takes no --file."));
    }
    Ok(body.map_or(Payload::None, Payload::Json))
}

fn body(operation: &Operation, args: &RunArgs) -> Result<Option<Value>> {
    let given = match &args.data {
        Some(data) => {
            let text = match data.as_str() {
                "-" => {
                    let mut text = String::new();
                    io::stdin()
                        .read_to_string(&mut text)
                        .map_err(|error| format!("stdin: {error}"))?;
                    text
                }
                data => match data.strip_prefix('@') {
                    Some(path) => {
                        fs::read_to_string(path).map_err(|error| file(path.as_ref(), error))?
                    }
                    None => data.to_owned(),
                },
            };
            let json = serde_json::from_str(&text)
                .map_err(|error| Error::usage(format!("-d isn't JSON: {error}")))?;
            Some(json)
        }
        None if args.fields.is_empty() => None,
        None => Some(Value::Object(Map::from_iter(args.fields.iter().cloned()))),
    };
    let Some(schema) = &operation.body else {
        return match given {
            Some(_) => Err(Error::usage("This command takes no body.")),
            None => Ok(None),
        };
    };
    let body = given.unwrap_or_else(|| Value::Object(Map::new()));
    if let Some(fields) = body.as_object() {
        let missing: Vec<String> = schema["required"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|required| !fields.contains_key(*required))
            .map(|required| format!("--{}", required.replace('_', "-")))
            .collect();
        if !missing.is_empty() {
            return Err(Error::usage(format!(
                "Missing {}. Add --help to see the flags.",
                missing.join(", ")
            )));
        }
    }
    Ok(Some(body))
}

pub fn send(api: &Api, operation: &Operation, args: &RunArgs, payload: Payload) -> Result<Answer> {
    let path = operation.fill(api.org, &args.params)?;
    let method = Method::from_bytes(operation.method.as_bytes())
        .map_err(|_| "This command uses a method this fob doesn't know. Update fob.")?;
    let request = api
        .client
        .request(method, format!("{}/api{path}", api.console))
        .bearer_auth(api.token)
        .query(&args.query);
    let request = match payload {
        Payload::None => request,
        Payload::Json(body) => request.json(&body),
        Payload::File(bytes) => request
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(bytes),
    };

    let response = request.send()?;
    let status = response.status();
    let retry_after = response
        .headers()
        .get(header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = response.bytes()?.to_vec();
    let json: Option<Value> = serde_json::from_slice(&bytes).ok();
    if status.is_success() {
        return Ok(Answer { bytes, json });
    }

    let mut message = json
        .as_ref()
        .and_then(|json| json["message"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("The server answered {status}."));
    if status == StatusCode::UNAUTHORIZED {
        message = format!("{message} {}", api.if_unauthorized);
    }
    if let Some(seconds) = retry_after {
        message.push_str(&format!(" Retry in {seconds}s."));
    }
    let exit = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => NOT_ALLOWED,
        StatusCode::NOT_FOUND => NOT_FOUND,
        StatusCode::CONFLICT => CONFLICT,
        StatusCode::UNPROCESSABLE_ENTITY | StatusCode::BAD_REQUEST => USAGE,
        _ => 1,
    };
    let code = json
        .as_ref()
        .and_then(|json| json["error"].as_str())
        .unwrap_or("error");
    Err(Error::new(code, exit, message))
}

/// Asks before deleting, when there's someone to ask.
pub fn confirm(operation: &Operation, args: &RunArgs) -> Result<()> {
    if operation.method != "DELETE" || args.yes {
        return Ok(());
    }
    if !io::stdin().is_terminal() {
        return Err(Error::usage("Add --yes to delete without being asked."));
    }
    eprint!("{} [y/N] ", operation.summary);
    let mut answer = String::new();
    let _ = io::stdin().read_line(&mut answer);
    match answer.trim() {
        "y" | "Y" | "yes" => Ok(()),
        _ => Err(Error::new("cancelled", 1, "Nothing was deleted.")),
    }
}

/// Prints an answer: laid out for a person in a terminal or with `--text`, as JSON
/// otherwise. A download is written as it came.
pub fn print(
    answer: &Answer,
    operation: &Operation,
    args: &RunArgs,
    global: &Global,
) -> Result<()> {
    let terminal = io::stdout().is_terminal();
    let readable = !global.json && args.output.is_none() && (terminal || global.text);
    let bytes = match &answer.json {
        _ if operation.download => answer.bytes.clone(),
        Some(json) if readable => {
            let no_color = std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty());
            let style = show::Style {
                color: terminal && !no_color,
                now: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|since| since.as_secs() as i64)
                    .unwrap_or(0),
            };
            show::readable(json, show::columns(&operation.name), &style).into_bytes()
        }
        Some(json) => format!("{json}\n").into_bytes(),
        None => {
            if readable && answer.bytes.is_empty() {
                eprintln!("Done.");
            }
            answer.bytes.clone()
        }
    };
    match &args.output {
        Some(path) => fs::write(path, bytes).map_err(|error| file(path, error)),
        None => {
            let _ = io::stdout().write_all(&bytes);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn operation(body: Option<Value>) -> Operation {
        Operation {
            name: "create_group".into(),
            group: "groups".into(),
            summary: String::new(),
            method: "POST".into(),
            path: "/orgs/{org}/groups".into(),
            query: None,
            body,
            upload: false,
            download: false,
        }
    }

    #[test]
    fn builds_a_body_from_fields() {
        let op = operation(Some(json!({ "required": ["name"] })));
        let args = RunArgs {
            fields: vec![
                ("name".into(), json!("Engineering")),
                ("size".into(), json!(3)),
            ],
            ..Default::default()
        };
        assert_eq!(
            body(&op, &args).unwrap(),
            Some(json!({ "name": "Engineering", "size": 3 }))
        );
    }

    #[test]
    fn names_missing_fields() {
        let op = operation(Some(json!({ "required": ["name", "kind"] })));
        let args = RunArgs {
            data: Some(r#"{"kind":"web"}"#.into()),
            ..Default::default()
        };
        let error = body(&op, &args).unwrap_err();
        assert_eq!(
            error.message,
            "Missing --name. Add --help to see the flags."
        );
        assert_eq!(error.exit, USAGE);
    }

    #[test]
    fn refuses_a_body_where_none_is_taken() {
        let args = RunArgs {
            data: Some("{}".into()),
            ..Default::default()
        };
        assert_eq!(body(&operation(None), &args).unwrap_err().exit, USAGE);
        assert_eq!(body(&operation(None), &RunArgs::default()).unwrap(), None);
    }

    #[test]
    fn an_upload_needs_its_file_and_nothing_else_takes_one() {
        let mut upload = operation(None);
        upload.upload = true;
        assert!(payload(&upload, &RunArgs::default()).is_err());
        let args = RunArgs {
            file: Some("logo.png".into()),
            ..Default::default()
        };
        assert!(payload(&operation(None), &args).is_err());
    }
}
