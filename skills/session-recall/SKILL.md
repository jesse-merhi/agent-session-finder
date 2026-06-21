---
name: session-recall
description: Find prior local Codex or Claude agent sessions with the agent-session-find CLI. Use when context may exist in previous sessions, after compaction or handoff, before asking the user to remember prior work, when searching for old fixes, decisions, commands, errors, file paths, repo context, review outcomes, or "where did we handle this before?" Keep session contents local and use low-token search results before opening full logs.
---

# Session Recall

Use `agent-session-find` as the first recall step when previous agent work may answer the current question. The goal is to recover the right full session with minimal tokens, not to dump transcripts into context.

## Command Setup

Prefer the installed binary:

```sh
agent-session-find --help
```

If it is unavailable and this checkout is present, use the wrapper:

```sh
./agent-session-find --help
```

Do not upload session contents or paste whole session logs into external tools. The CLI reads local Codex and Claude JSONL stores and writes a local SQLite FTS index.

If the default index path is not writable in the current sandbox, set an explicit local database:

```sh
agent-session-find --db /private/tmp/session-recall.sqlite --index-since 14d --max-sources 80 "<query>"
```

Do not run parallel refreshes against the same SQLite database. Use sequential searches, `--no-refresh` after the first refresh, or separate `--db` paths.

## Recall Workflow

1. Start with a recent, bounded fuzzy query.

```sh
agent-session-find --index-since 14d --max-sources 80 "example_repo export ui"
```

2. Add repo or cwd context when the project is known.

```sh
agent-session-find --cwd example_repo --since 30d "export bugs"
```

3. Search one source when the likely harness is known.

```sh
agent-session-find --source codex "review state tracker"
agent-session-find --source claude "mobile build workflow"
```

4. If nothing matches, widen gradually: increase `--index-since`, remove `--cwd`, try synonyms, then omit `--max-sources` for a fuller local refresh.

5. If running several follow-up searches against the same index, add `--no-refresh` after the first successful refresh.

6. Inspect the low-token result cards first. Prefer results with matching `cwd`, `repo`, `terms`, recent timestamp, and `session: full`.

7. Open or grep the source path only after choosing a likely result. Pull only the narrow lines needed for the task.

## Query Strategy

Use the words the user or agent likely typed, not a perfect summary. Good query terms include:

- product or repo names: `example_repo`, `agent-session-find`
- visible feature words: `export ui`, `mobile build`, `database restore`
- error text or symbols: `missing_symbol`, `No such file`
- workflow labels: `code review`, `test-audit`, `installer`
- file or command fragments: `Cargo.toml`, `install.sh`, `bun run check`

Run two or three short searches instead of one long paragraph. Keep exact phrases for rare terms and use broader words for fuzzy recall.

## Result Handling

Treat search results as routing metadata:

- `title` tells whether the session topic sounds right.
- `match` and `terms` show why it matched.
- `cwd` and `repo` tell whether it belongs to the current codebase.
- `session id` and `source` point to the local JSONL if deeper inspection is needed.
- snippets are enough for most routing decisions.

Do not assume a result proves the old decision is still correct. Use it to find context, then verify current code or docs before acting.

## Defaults

Use these defaults unless the task suggests otherwise:

```sh
agent-session-find --index-since 14d --max-sources 80 "<query>"
agent-session-find --cwd "<repo-name>" --since 30d "<query>"
agent-session-find --limit 5 "<query>"
```

For stale or long-running projects, prefer `--index-since 90d` over an unbounded first pass. Run `agent-session-find status` when you need to see index size before widening.
