# fob

Command line client for [Fob](https://fobauth.com). It runs every operation of the Fob API.

## Install

Linux and macOS:

```sh
curl -fsSL https://fob.so/install.sh | sh
```

Windows (PowerShell):

```powershell
irm https://fob.so/install.ps1 | iex
```

Both download the latest [release](https://github.com/fobauth/cli/releases/latest) and check it against the release's `SHA256SUMS` before installing. `install.sh` installs to `~/.local/bin`, or to `FOB_INSTALL_DIR` if you set it. `install.ps1` installs to `%LOCALAPPDATA%\Programs\fob` and adds that to your PATH.

To build from source you need Rust 1.89 or newer and a C compiler:

```sh
cargo install --git https://github.com/fobauth/cli
```

## Sign in

```sh
fob login --org acme
```

Approve the code in the browser page it opens. With `--no-browser` it only prints the address. `fob` remembers the organization, so later commands need no `--org`. `fob whoami` shows the organization and your role, and `fob logout` signs out.

In scripts, use an API key from the console instead:

```sh
FOB_API_KEY=fob_sk_... fob groups list --org acme
```

## Use

```sh
fob                                # every group and its commands
fob groups                         # one group's commands
fob groups create --help           # a command's usage, flags and an example
fob search password                # find a command by words

fob groups create --name Engineering --description "Builds things"
fob groups get Engineering
fob people list --search ada --limit 10
fob apps create -d @app.json
fob branding upload-logo light --file logo.png
fob people export -o people.csv
fob groups delete Engineering --yes
```

- IDs come first, in the order `--help` shows. For people, groups, apps, policies, networks, webhooks, identity providers, API keys and roles you can type a name or email instead of the ID.
- Every field of a command is a `--flag`. `-d` takes the whole body as JSON instead: inline, `@file`, or `-` for stdin.
- Deleting asks first. `--yes` skips the question, and is required when there is no terminal to ask in.
- `--refresh` refetches the list of commands, which is otherwise kept for an hour.

## Output

In a terminal, answers print as aligned fields and tables. Piped, they are JSON.

| Flag | |
| --- | --- |
| `--json` | JSON everywhere, with errors as `{"error", "message"}` on stderr |
| `--text` | The readable layout, even when piped |

`--json` wins if both are given. Colour is only used in a terminal, and not when `NO_COLOR` is set.

| Exit status | |
| --- | --- |
| 0 | Done |
| 1 | Failed |
| 2 | Typed wrong, or refused as invalid |
| 3 | Not found |
| 4 | Not signed in, or not allowed |
| 5 | Conflict |

## Environment

| Variable | Flag | |
| --- | --- | --- |
| `FOB_ORG` | `--org` | Organization slug |
| `FOB_API_KEY` | `--api-key` | API key, instead of signing in. Needs `--org`. |
| `FOB_HOST` | `--host` | Base domain. Default `fobauth.com`. For a local server, `localhost:8080`. |
| `FOB_CONSOLE` | `--console` | Console URL, when it isn't the default for the host |

## Develop

```sh
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
```

To release, set `version` in `Cargo.toml`, run `cargo build` so `Cargo.lock` follows, commit, then tag that commit `v<version>` and push the tag. The release workflow builds the binaries and publishes them with `SHA256SUMS`.
