mod args;
mod auth;
mod catalogue;
mod error;
mod hosts;
mod run;
mod show;
mod store;

use std::{
    io::{self, Write},
    process::ExitCode,
};

use clap::{Parser, Subcommand};
use serde::Serialize;
use serde_json::json;

use crate::{
    args::{Global, RunArgs},
    catalogue::Operation,
    error::{Error, NOT_FOUND, Result},
    hosts::Hosts,
    run::{Answer, Payload},
    store::Config,
};

#[derive(Parser)]
#[command(
    name = "fob",
    bin_name = "fob",
    version,
    disable_help_flag = true,
    disable_help_subcommand = true
)]
struct Cli {
    #[command(flatten)]
    global: Global,
    #[arg(short, long, global = true, hide = true)]
    help: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Login {
        #[arg(long)]
        no_browser: bool,
    },
    Logout,
    Whoami,
    Search {
        words: Vec<String>,
    },
    Help,
    /// A group and one of its commands, with everything typed after them.
    #[command(external_subcommand)]
    Api(Vec<String>),
}

struct Session {
    global: Global,
    config: Config,
}

impl Session {
    fn hosts(&self) -> Hosts {
        let host = self
            .global
            .host
            .as_deref()
            .or(self.config.host.as_deref())
            .unwrap_or(hosts::HOSTED_DOMAIN);
        Hosts::new(host, self.global.console.as_deref())
    }

    fn org(&self) -> Result<String> {
        let org = match (&self.global.org, &self.global.api_key) {
            (Some(org), _) => org.clone(),
            (None, Some(_)) => return Err("Pass --org <slug> with an API key.".into()),
            (None, None) => self
                .config
                .org
                .clone()
                .ok_or("Pass --org <slug>, or run `fob login --org <slug>`.")?,
        };
        let valid = !org.is_empty()
            && org
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !valid {
            return Err(format!("{org} isn't an organization slug.").into());
        }
        Ok(org)
    }

    fn operations(&self) -> Result<Vec<Operation>> {
        let hosts = self.hosts();
        let client = hosts::client(&[&hosts.console]);
        catalogue::load(&client, &hosts.console, self.global.refresh)
    }

    fn send(&self, operation: &Operation, args: &RunArgs, payload: Payload) -> Result<Answer> {
        let hosts = self.hosts();
        let org = self.org()?;
        let tenant = hosts.tenant(&org);
        let client = hosts::client(&[&hosts.console, &tenant]);
        let (token, if_unauthorized) = match &self.global.api_key {
            Some(key) => (key.clone(), "Check the API key and --org.".to_owned()),
            None => (
                auth::access_token(&client, &tenant, &org)?,
                format!("Run `fob login --org {org}`."),
            ),
        };
        let api = run::Api {
            client: &client,
            console: &hosts.console,
            org: &org,
            token: &token,
            if_unauthorized: &if_unauthorized,
        };
        run::send(&api, operation, args, payload)
    }
}

fn main() -> ExitCode {
    let json_errors = std::env::args().any(|arg| arg == "--json");
    match fob() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if json_errors {
                eprintln!(
                    "{}",
                    json!({ "error": error.code, "message": error.message })
                );
            } else {
                eprintln!("{error}");
            }
            ExitCode::from(error.exit)
        }
    }
}

fn fob() -> Result<()> {
    let cli = Cli::parse();
    let mut global = cli.global;
    // A variable set to nothing, as in `FOB_ORG= fob ...`, counts as not set.
    for value in [
        &mut global.org,
        &mut global.api_key,
        &mut global.host,
        &mut global.console,
    ] {
        if value.as_deref().is_some_and(|set| set.trim().is_empty()) {
            *value = None;
        }
    }
    let mut session = Session {
        global,
        config: Config::load(),
    };
    match cli.command {
        None | Some(Command::Help) => overview(&session),
        Some(Command::Api(words)) => api(&mut session, &words),
        Some(_) if cli.help => overview(&session),
        Some(Command::Login { no_browser }) => login(&mut session, !no_browser),
        Some(Command::Logout) => logout(&session),
        Some(Command::Whoami) => whoami(&session),
        Some(Command::Search { words }) => search(&session, &words),
    }
}

fn login(session: &mut Session, browser: bool) -> Result<()> {
    let hosts = session.hosts();
    let org = session.org()?;
    let tenant = hosts.tenant(&org);
    auth::login(&hosts::client(&[&tenant]), &tenant, &org, browser)?;
    if let Some(host) = &session.global.host {
        session.config.host = Some(host.clone());
    }
    session.config.org = Some(org.clone());
    session.config.save()?;
    eprintln!("Signed in to {org}.");
    Ok(())
}

fn logout(session: &Session) -> Result<()> {
    let org = session.org()?;
    let tenant = session.hosts().tenant(&org);
    if auth::logout(&hosts::client(&[&tenant]), &tenant)? {
        eprintln!("Signed out of {org}.");
    } else {
        eprintln!("Not signed in to {org}.");
    }
    Ok(())
}

fn whoami(session: &Session) -> Result<()> {
    let operations = session.operations()?;
    let operation = catalogue::find(&operations, "get_organization")?;
    let organization = session
        .send(operation, &RunArgs::default(), Payload::None)?
        .json
        .unwrap_or_default();
    if session.global.json {
        print_json(&organization);
        return Ok(());
    }
    let text = |field: &str| organization[field].as_str().unwrap_or("?");
    let role = organization["custom_role"]["name"]
        .as_str()
        .unwrap_or_else(|| text("role"));
    out(&format!(
        "{} ({})\nRole: {role}\n",
        text("name"),
        text("slug")
    ));
    Ok(())
}

fn search(session: &Session, words: &[String]) -> Result<()> {
    let operations = session.operations()?;
    let found = catalogue::search(&operations, words);
    if session.global.json {
        print_json(&found);
    } else if found.is_empty() {
        return Err("No commands match.".into());
    } else {
        list(&operations, &found, true);
    }
    Ok(())
}

/// `fob <group>` lists the group's commands. `fob <group> <command> ...` runs one.
fn api(session: &mut Session, words: &[String]) -> Result<()> {
    let operations = session.operations()?;
    let group = catalogue::in_group(&operations, &words[0]);
    if !group.is_empty() && words.get(1).is_none_or(|next| next.starts_with('-')) {
        if session.global.json || words.iter().any(|word| word == "--json") {
            print_json(&group);
        } else {
            list(&operations, &group, false);
            eprintln!("\nfob {} <command> --help shows what one takes.", words[0]);
        }
        return Ok(());
    }

    let (operation, used) = catalogue::from_words(&operations, words)?;
    let mut args = args::parse(operation, &words[used..], &mut session.global)?;
    if args.help {
        let typed = format!(
            "{} {}",
            operation.group,
            catalogue::command(&operations, operation)
        );
        out(&operation.usage(&typed));
        return Ok(());
    }
    let payload = run::payload(operation, &args)?;
    for (param, typed) in args.params.clone() {
        if let Some(id) = named(session, &operations, &param, &typed)? {
            args.params.insert(param, id);
        }
    }
    run::confirm(operation, &args)?;
    let answer = session.send(operation, &args, payload)?;
    run::print(&answer, operation, &args, &session.global)
}

/// The list to look in for each kind of thing that can be typed by name.
const LISTS: &[(&str, &str)] = &[
    ("person", "list_people"),
    ("group", "list_groups"),
    ("app", "list_apps"),
    ("policy", "list_sign_on_policies"),
    ("network", "list_networks"),
    ("webhook", "list_webhooks"),
    ("provider", "list_identity_providers"),
    ("key", "list_api_keys"),
    ("role", "list_roles"),
];

/// The ID of the thing typed by its name or email, when it wasn't typed as an ID.
fn named(
    session: &Session,
    operations: &[Operation],
    param: &str,
    typed: &str,
) -> Result<Option<String>> {
    if show::is_id(typed) {
        return Ok(None);
    }
    let list = LISTS
        .iter()
        .find(|(kind, _)| *kind == param)
        .and_then(|(_, name)| operations.iter().find(|operation| operation.name == *name));
    let Some(list) = list else {
        return Ok(None);
    };
    let mut args = RunArgs::default();
    let searches = list
        .query
        .as_ref()
        .is_some_and(|schema| schema["properties"].get("search").is_some());
    if searches {
        args.query.push(("search".into(), typed.to_owned()));
    }
    let answer = session.send(list, &args, Payload::None)?;
    let json = answer.json.unwrap_or_default();
    let ids: Vec<&str> = show::list(&json)
        .iter()
        .filter(|row| {
            ["name", "email", "url", "slug"].iter().any(|field| {
                row[*field]
                    .as_str()
                    .is_some_and(|value| value.eq_ignore_ascii_case(typed))
            })
        })
        .filter_map(|row| row["id"].as_str().or(row["user_id"].as_str()))
        .collect();
    match ids.as_slice() {
        [id] => Ok(Some((*id).to_owned())),
        [] => Err(Error::new(
            "not_found",
            NOT_FOUND,
            format!("No {param} called {typed}."),
        )),
        several => Err(Error::usage(format!(
            "{} {param}s are called {typed}. Use an ID: {}.",
            several.len(),
            several.join(", ")
        ))),
    }
}

/// Writes to stdout. A reader that has gone, as in `fob | head`, is not an error.
fn out(text: &str) {
    let _ = io::stdout().write_all(text.as_bytes());
}

fn print_json(value: &impl Serialize) {
    let json = serde_json::to_string(value).expect("JSON serializes");
    out(&format!("{json}\n"));
}

const OVERVIEW_END: &str = "
Other commands:
  login               Sign in to an organization
  logout              Sign out
  whoami              Show the organization and your role
  search <words>      Find a command

Flags:
  --org <slug>        Organization, when it isn't the one you signed in to
  --api-key <key>     Use an API key instead of signing in
  --host <domain>     Base domain, such as localhost:8080
  --json              Print JSON, and errors as JSON
  --text              Print the readable layout, even when piped
  --help              Show what a group or a command takes

Examples:
  fob groups                       List what you can do with groups
  fob groups create --help         Show what one takes
  fob groups create --name Engineering
  fob people list --search ada
";

fn overview(session: &Session) -> Result<()> {
    let operations = session.operations()?;
    if session.global.json {
        print_json(&operations);
        return Ok(());
    }
    let mut text =
        "Manage a Fob organization.\n\nUsage: fob <group> <command> [flags]\n\nGroups:\n"
            .to_owned();
    for name in catalogue::groups(&operations) {
        let commands: Vec<String> = catalogue::in_group(&operations, name)
            .into_iter()
            .map(|operation| catalogue::command(&operations, operation))
            .collect();
        let more = if commands.len() > 6 { ", …" } else { "" };
        let shown = &commands[..commands.len().min(6)];
        text.push_str(&format!("  {name:18}  {}{more}\n", shown.join(", ")));
    }
    text.push_str(OVERVIEW_END);
    out(&text);
    Ok(())
}

fn list(all: &[Operation], shown: &[&Operation], with_group: bool) {
    let rows: Vec<(String, &str)> = shown
        .iter()
        .map(|operation| {
            let command = catalogue::command(all, operation);
            let command = if with_group {
                format!("{} {command}", operation.group)
            } else {
                command
            };
            (command, operation.summary.as_str())
        })
        .collect();
    let width = rows
        .iter()
        .map(|(command, _)| command.len())
        .max()
        .unwrap_or(0);
    let lines: String = rows
        .iter()
        .map(|(command, summary)| format!("{command:width$}  {summary}\n"))
        .collect();
    out(&lines);
}
