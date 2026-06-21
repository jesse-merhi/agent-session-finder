# Local Agent Session Finder

Local-first CLI for finding Codex and Claude sessions by prompt text, repo/cwd, file paths, and real errors.

It reads local stores only:

- `~/.codex/sessions/**/*.jsonl`
- `~/.codex/archived_sessions/**/*.jsonl`
- `~/.claude/transcripts/*.jsonl`
- `~/.claude/projects/*/*.jsonl`

Session contents stay on the machine. The index is a local SQLite FTS5 database at `~/.agent-session-finder.sqlite` by default.

The index is compact by design. It stores metadata, each user prompt with the next 3 assistant/tool messages, and short standalone failure snippets. It skips giant successful tool output.

## Install

```sh
./install.sh
```

That builds the Rust release binaries and copies `agent-session-find` plus `agent-skill-validate` to `~/.local/bin`. If that directory is not on your `PATH`, the installer prints the line to add.

To install somewhere else:

```sh
./install.sh --bin-dir /usr/local/bin
./install.sh --prefix "$HOME/.cargo"
```

From this checkout, you can also run the local wrapper directly. It prefers an already-built Rust binary and falls back to `cargo run`.

```sh
./agent-session-find --index-since 2d --max-sources 80 --no-logs "example_repo export ui"
```

Validate the bundled recall skill without Python or PyYAML:

```sh
./agent-skill-validate skills/session-recall
```

## Search

```sh
./agent-session-find "restore db walk me through"
./agent-session-find --index-since 14d "example_repo export ui"
./agent-session-find --source claude "mobile build workflow"
./agent-session-find --source codex "review state tracker"
```

Results are deliberately low-token: title, match count, session kind, matched terms, cwd, session id, source rollout path, and one or two snippets. Codex subagent/worker sessions are excluded from indexing and search by default.

## Index

```sh
./agent-session-find index
./agent-session-find index --index-since 30d
./agent-session-find index --source claude
```

For a bounded first run:

```sh
./agent-session-find index --index-since 7d --max-sources 100
```

## Status

```sh
./agent-session-find status
```

## Background Indexing

Manual searches auto-index an empty database, and `index` can be run from cron or launchd.
