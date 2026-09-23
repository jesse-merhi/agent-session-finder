# Agent Session Finder

`agent-session-find` is a local CLI for developers who need to find Codex and Claude sessions by prompt text, repository, working directory, file path, or error message.

## Why

Agent histories are split across timestamped JSONL files and are difficult to search by memory. The finder builds a compact SQLite FTS5 index and returns low-token result cards; the companion `agent-skill-validate` binary checks local Codex skill folders without Python or PyYAML.

## Install

### Prebuilt binaries

Choose one release target: `aarch64-apple-darwin`, `x86_64-apple-darwin`, or `x86_64-unknown-linux-gnu`.

```sh
VERSION=v0.1.0
TARGET=aarch64-apple-darwin
ARCHIVE="agent-session-finder-${VERSION}-${TARGET}.tar.gz"
curl -LO "https://github.com/jesse-merhi/agent-session-finder/releases/download/${VERSION}/${ARCHIVE}"
curl -LO "https://github.com/jesse-merhi/agent-session-finder/releases/download/${VERSION}/SHA256SUMS"
grep " ${ARCHIVE}$" SHA256SUMS | shasum -a 256 -c -
tar -xzf "$ARCHIVE"
install -m 0755 "agent-session-finder-${VERSION}-${TARGET}/agent-session-find" "$HOME/.local/bin/"
install -m 0755 "agent-session-finder-${VERSION}-${TARGET}/agent-skill-validate" "$HOME/.local/bin/"
```

Replace `VERSION` with an available release tag and `TARGET` with your platform.

### Build with Cargo

```sh
cargo install --locked --git https://github.com/jesse-merhi/agent-session-finder
```

From a clone, `./install.sh` builds both binaries and installs them to `$HOME/.local/bin`. Use `./install.sh --help` for alternate destinations.

## Usage

Search all local sessions; an empty index is refreshed automatically:

```text
$ agent-session-find "login redirect"
1. Repair Google sign-in redirect
   match: 2/2  kind: user_prompt,assistant_message  score: 18.4
   session: full
   terms: login,redirect
   cwd: /Users/alex/repos/web-app
   id:  8b6a9f44-1c37-4fc3-8f21-9f28a76c13f4
   src: /Users/alex/.codex/sessions/2026/08/03/rollout-...jsonl
```

Build or inspect the index explicitly:

```sh
agent-session-find index --index-since 14d --max-sources 100
agent-session-find status
agent-session-find --source claude --cwd web-app "mobile build"
agent-session-find --workers "eslint review"
agent-skill-validate skills/session-recall
```

No match is a non-zero exit so scripts can distinguish it from success.

After selecting a session, read excerpts from its `src` path without opening or
refreshing the index:

```sh
agent-session-find --read /path/to/session.jsonl 'requirements.toml'
agent-session-find --compact-docs /path/to/saved-doc-search.json --max-bytes 4096
```

Both modes return JSON with one total byte budget (8,192 by default), source
references, and `next_offset` for another page. Pass that offset with the same
input and query. Transcript matching uses a case-sensitive literal and keeps
matches inside long messages; an omitted query shows the start of each message.
`truncated` marks shortened passages. Pagination advances between excerpts, so
use a more specific query to inspect the hidden part of a message. Documentation
input can be the search JSON itself or an MCP `content`/`structuredContent`
response. Cards retain full URLs and project titles and short excerpts instead
of repeating full page content and highlights. The `search` field retains
upstream pagination separately from pagination through the supplied hits.

Read modes leave input files unchanged and never access the session index.
An empty page is a successful read; invalid input or an item that cannot fit
the requested budget exits nonzero without partial JSON. See `--help` for limits.

## Configuration

Environment variables:

| Variable | Default | Purpose |
| --- | --- | --- |
| `AGENT_SESSION_FINDER_CODEX_HOME` | `$CODEX_HOME` or `$HOME/.codex` | Codex session store. |
| `CODEX_HOME` | `$HOME/.codex` | Native Codex store override. |
| `AGENT_SESSION_FINDER_CLAUDE_HOME` | `$CLAUDE_CONFIG_DIR` or `$HOME/.claude` | Claude session store. |
| `CLAUDE_CONFIG_DIR` | `$HOME/.claude` | Native Claude store override. |
| `AGENT_SESSION_FINDER_DB` | see below | SQLite index path. |
| `XDG_CACHE_HOME` | unset | When set, the default index is `$XDG_CACHE_HOME/agent-session-finder/index.sqlite`; otherwise it is `$HOME/.agent-session-finder.sqlite`. |
| `HOME` | system home | Base for default stores and index. |
| `BIN_DIR` | unset | Installer destination directory. |
| `PREFIX` | unset | Installer prefix; binaries go in `PREFIX/bin`. |
| `PROFILE` | `release` | Installer build profile: `release` or `debug`. |
| `CARGO_TARGET_DIR` | Cargo default | Alternate build-output directory used by the installer. |

`agent-session-find` flags:

| Flag | Purpose |
| --- | --- |
| `--codex-home PATH` | Override the Codex store. |
| `--claude-home PATH` | Override the Claude store. |
| `--db PATH` | Override the SQLite index. |
| `--source all\|codex\|claude` | Select stores; default is `all`. |
| `--cwd TEXT` | Restrict matches by working directory or repository. |
| `--since 6h\|2d\|1w` | Restrict results by age. |
| `--index-since 6h\|2d\|1w` | Restrict indexed source files by age. |
| `--max-sources N` | Bound the source files processed. |
| `--workers` | Include Codex worker/subagent sessions. `--include-workers` and `--include-subagents` are aliases. |
| `--no-archived` | Exclude archived Codex sessions. |
| `--no-refresh` | Search the existing index without refreshing. |
| `--no-logs` | Accepted as a deprecated no-op for older scripts. |
| `--limit N` | Set the maximum result count; default is `10`. |
| `-h`, `--help` | Show help. |
| `-V`, `--version` | Show version. |

`agent-skill-validate` accepts skill folders, `SKILL.md` files, or directories containing skills. With no path it checks `./skills`; its flags are `-h`/`--help` and `-V`/`--version`.

`install.sh` accepts `--bin-dir DIR`, `--prefix DIR`, `--profile release|debug`, `--no-build`, and `-h`/`--help`. Matching environment variables are listed above.

## Privacy and local-only behavior

Session files and the SQLite index stay on your machine. The program makes no network requests. It indexes metadata, user prompts with a small following context window, compact worker handoff summaries, and short failure snippets while skipping large successful tool output. Worker transcripts are excluded unless `--workers` is set.

## Requirements

- macOS on Apple Silicon or Intel, or x86-64 Linux, for the supplied archives
- a current stable Rust toolchain when building from source
- readable local Codex and/or Claude session stores
- a C toolchain when compiling bundled SQLite from source

## License

MIT. See [LICENSE](LICENSE).
