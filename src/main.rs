use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::env;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

mod evidence;

const MAX_BODY_CHARS: usize = 6_000;
const CONTEXT_DOCS: usize = 3;
const MAX_STANDALONE_ERRORS: usize = 5;
const ERROR_CHARS: usize = 1_200;
const INDEX_VERSION: &str = "2";

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, Clone)]
struct Config {
    codex_home: PathBuf,
    claude_home: PathBuf,
    db: PathBuf,
    source: Source,
    command: CommandKind,
    query: String,
    cwd: Option<String>,
    no_refresh: bool,
    no_archived: bool,
    include_workers: bool,
    limit: usize,
    max_sources: Option<usize>,
    index_since: Option<Duration>,
    since: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    All,
    Codex,
    Claude,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandKind {
    Search,
    Index,
    Status,
}

#[derive(Debug, Clone)]
struct RawDoc {
    session_id: String,
    source_kind: String,
    source_path: String,
    session_kind: String,
    timestamp: String,
    title: String,
    cwd: String,
    repo: String,
    role: String,
    category: String,
    body: String,
}

#[derive(Debug, Clone)]
struct SessionMeta {
    session_id: String,
    title: String,
    cwd: String,
    updated_at: String,
    source_path: String,
    session_kind: String,
    parent_thread_id: String,
    repo: String,
}

#[derive(Debug)]
struct SearchRow {
    session_id: String,
    title: String,
    cwd: String,
    updated_at: String,
    source_path: String,
    session_kind: String,
    parent_thread_id: String,
    category: String,
    body: String,
    rank: f64,
}

#[derive(Debug)]
struct ResultCard {
    session_id: String,
    title: String,
    cwd: String,
    updated_at: String,
    source_path: String,
    session_kind: String,
    parent_thread_id: String,
    categories: BTreeSet<String>,
    snippets: Vec<String>,
    matched_terms: BTreeSet<String>,
    best_row_terms: usize,
    rank: f64,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> AppResult<()> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let options_end = args
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(args.len());
    let option_args = &args[..options_end];
    if option_args.iter().any(|arg| arg == "-h" || arg == "--help") {
        print_help()?;
        return Ok(());
    }
    if option_args
        .iter()
        .any(|arg| arg == "-V" || arg == "--version")
    {
        println!("agent-session-find {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if evidence::run_if_requested(&args)? {
        return Ok(());
    }
    let config = parse_args(args)?;
    if config.command == CommandKind::Index {
        validate_source_stores(&config)?;
    }
    let mut conn = open_db(&config.db)?;

    match config.command {
        CommandKind::Index => {
            let summary = index_all(&mut conn, &config)?;
            let mut stdout = io::stdout().lock();
            write_io(writeln!(
                stdout,
                "Indexed {} changed sources, skipped {} unchanged sources, filtered {} sources, {} searchable docs.",
                summary.indexed, summary.skipped, summary.filtered, summary.docs
            ))?;
            write_io(writeln!(stdout, "Index: {}", config.db.display()))?;
            Ok(())
        }
        CommandKind::Status => {
            let sessions: i64 =
                conn.query_row("select count(*) from sessions", [], |row| row.get(0))?;
            let docs: i64 = conn.query_row("select count(*) from docs", [], |row| row.get(0))?;
            let sources: i64 =
                conn.query_row("select count(*) from sources", [], |row| row.get(0))?;
            let mut stdout = io::stdout().lock();
            write_io(writeln!(stdout, "Index: {}", config.db.display()))?;
            write_io(writeln!(stdout, "Sessions: {sessions}"))?;
            write_io(writeln!(stdout, "Docs: {docs}"))?;
            write_io(writeln!(stdout, "Sources: {sources}"))?;
            Ok(())
        }
        CommandKind::Search => {
            if !config.no_refresh && source_scope_needs_auto_index(&conn, &config)? {
                validate_source_stores(&config)?;
                index_all(&mut conn, &config)?;
            }
            let results = search(&conn, &config)?;
            if results.is_empty() {
                let mut stdout = io::stdout().lock();
                write_io(writeln!(
                    stdout,
                    "No matching sessions found (query='{}').",
                    config.query
                ))?;
                std::process::exit(1);
            }
            print_results(&results, &config.query, config.limit)?;
            Ok(())
        }
    }
}

#[derive(Debug)]
struct IndexSummary {
    indexed: usize,
    skipped: usize,
    filtered: usize,
    docs: i64,
}

fn parse_args(args: Vec<String>) -> AppResult<Config> {
    let mut config = Config {
        codex_home: default_store_path("AGENT_SESSION_FINDER_CODEX_HOME", "CODEX_HOME", ".codex")?,
        claude_home: default_store_path(
            "AGENT_SESSION_FINDER_CLAUDE_HOME",
            "CLAUDE_CONFIG_DIR",
            ".claude",
        )?,
        db: default_db_path()?,
        source: Source::All,
        command: CommandKind::Search,
        query: String::new(),
        cwd: None,
        no_refresh: false,
        no_archived: false,
        include_workers: false,
        limit: 10,
        max_sources: None,
        index_since: None,
        since: None,
    };

    let mut query_parts = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-h" | "--help" => {
                print_help()?;
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("agent-session-find {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--codex-home" => {
                index += 1;
                config.codex_home = expand_home(&required_arg(&args, index, "--codex-home")?);
            }
            "--claude-home" => {
                index += 1;
                config.claude_home = expand_home(&required_arg(&args, index, "--claude-home")?);
            }
            "--db" => {
                index += 1;
                config.db = expand_home(&required_arg(&args, index, "--db")?);
            }
            "--source" => {
                index += 1;
                config.source = match required_arg(&args, index, "--source")?.as_str() {
                    "all" => Source::All,
                    "codex" => Source::Codex,
                    "claude" => Source::Claude,
                    other => return Err(format!("unknown source: {other}").into()),
                };
            }
            "--no-refresh" => config.no_refresh = true,
            "--no-archived" => config.no_archived = true,
            "--workers" | "--include-workers" | "--include-subagents" => {
                config.include_workers = true
            }
            "--no-logs" => {}
            "--cwd" => {
                index += 1;
                config.cwd = Some(required_arg(&args, index, "--cwd")?);
            }
            "--limit" => {
                index += 1;
                config.limit = required_arg(&args, index, "--limit")?.parse()?;
            }
            "--max-sources" => {
                index += 1;
                config.max_sources = Some(required_arg(&args, index, "--max-sources")?.parse()?);
            }
            "--index-since" => {
                index += 1;
                let value = required_arg(&args, index, "--index-since")?;
                config.index_since = Some(parse_duration(&value, "--index-since")?);
            }
            "--since" => {
                index += 1;
                let value = required_arg(&args, index, "--since")?;
                config.since = Some(parse_duration(&value, "--since")?);
            }
            "index" if query_parts.is_empty() => config.command = CommandKind::Index,
            "status" if query_parts.is_empty() => config.command = CommandKind::Status,
            other if other.starts_with('-') => {
                return Err(format!("unknown option: {other}; run --help for usage").into())
            }
            other => query_parts.push(other.to_string()),
        }
        index += 1;
    }
    config.query = query_parts.join(" ");
    if config.limit == 0 {
        return Err("--limit must be greater than zero".into());
    }
    if config.max_sources == Some(0) {
        return Err("--max-sources must be greater than zero".into());
    }
    if config.command == CommandKind::Search
        && config.query.is_empty()
        && config.cwd.as_deref().unwrap_or_default().is_empty()
    {
        return Err("a search query or --cwd filter is required; run --help for usage".into());
    }
    Ok(config)
}

fn required_arg(args: &[String], index: usize, flag: &str) -> AppResult<String> {
    args.get(index)
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value").into())
}

fn print_help() -> AppResult<()> {
    let mut stdout = io::stdout().lock();
    write_io(writeln!(
        stdout,
        "usage: agent-session-find [OPTIONS] QUERY...\n       agent-session-find [OPTIONS] index|status"
    ))?;
    write_io(writeln!(stdout))?;
    write_io(writeln!(
        stdout,
        "Local lightweight session finder for Codex and Claude logs."
    ))?;
    write_io(writeln!(stdout, "\nRead local evidence without opening the index:\n  --read PATH [TEXT]      Read Codex/Claude JSONL; TEXT is a case-sensitive literal\n  --compact-docs PATH     Project saved documentation search JSON or an MCP response\n  --max-bytes N           Complete JSON output budget, including newline (default: 8192; minimum: 1024)\n  --offset N              Resume at next_offset with the same file and query (default: 0)\n  --limit N               Maximum excerpts/hits per page (default: 10)\n\nRead output has source references, truncated markers and next_offset (null at EOF).\nTranscript excerpts contain up to 480 UTF-8 bytes around each match, or the start\nof each message when TEXT is omitted. --offset pages excerpts, not the hidden\ntail of an individual excerpt; narrow TEXT to inspect another passage.\nDocumentation cards contain title, full URL and up to 240 excerpt bytes; search\nmetadata preserves upstream pagination separately from local next_offset.\n\nExamples:\n  agent-session-find --read /path/from/search.jsonl 'requirements.toml'\n  agent-session-find --read /path/from/search.jsonl --offset 10 'requirements.toml'\n  agent-session-find --compact-docs /path/to/saved-search.json --max-bytes 4096"))?;
    write_io(writeln!(
        stdout,
        "\nOptions:\n  --codex-home PATH       Codex store (default: $CODEX_HOME or ~/.codex)\n  --claude-home PATH      Claude store (default: $CLAUDE_CONFIG_DIR or ~/.claude)\n  --db PATH               SQLite index path\n  --source SOURCE         all, codex, or claude (default: all)\n  --cwd TEXT              Restrict matches by working directory\n  --since DURATION        Restrict results by age (for example 6h, 2d, 1w)\n  --index-since DURATION  Restrict indexing by source age\n  --max-sources N         Bound sources processed during indexing\n  --workers               Include worker/subagent sessions\n  --no-archived           Exclude archived Codex sessions\n  --no-refresh            Search the existing index without refreshing\n  --limit N               Maximum results (default: 10)\n  -h, --help              Show help\n  -V, --version           Show version"
    ))?;
    Ok(())
}

fn default_store_path(
    primary_env: &str,
    native_env: &str,
    home_relative: &str,
) -> AppResult<PathBuf> {
    if let Some(path) = env::var_os(primary_env).or_else(|| env::var_os(native_env)) {
        return Ok(PathBuf::from(path));
    }
    let home = env::var_os("HOME").ok_or_else(|| {
        format!("HOME is not set; set HOME or {primary_env} to the session store path")
    })?;
    Ok(PathBuf::from(home).join(home_relative))
}

fn default_db_path() -> AppResult<PathBuf> {
    if let Some(path) = env::var_os("AGENT_SESSION_FINDER_DB") {
        return Ok(PathBuf::from(path));
    }
    if let Some(cache_home) = env::var_os("XDG_CACHE_HOME") {
        return Ok(PathBuf::from(cache_home)
            .join("agent-session-finder")
            .join("index.sqlite"));
    }
    let home = env::var_os("HOME")
        .ok_or_else(|| "HOME is not set; set HOME or AGENT_SESSION_FINDER_DB".to_string())?;
    Ok(PathBuf::from(home).join(".agent-session-finder.sqlite"))
}

fn parse_duration(value: &str, flag: &str) -> AppResult<Duration> {
    let Some((unit_index, unit)) = value.char_indices().last() else {
        return Err(invalid_duration(flag, value).into());
    };
    let number = &value[..unit_index];
    let amount: u64 = number.parse().map_err(|_| invalid_duration(flag, value))?;
    match unit {
        'h' => Ok(Duration::from_secs(
            amount
                .checked_mul(60 * 60)
                .ok_or_else(|| invalid_duration(flag, value))?,
        )),
        'd' => Ok(Duration::from_secs(
            amount
                .checked_mul(24 * 60 * 60)
                .ok_or_else(|| invalid_duration(flag, value))?,
        )),
        'w' => Ok(Duration::from_secs(
            amount
                .checked_mul(7 * 24 * 60 * 60)
                .ok_or_else(|| invalid_duration(flag, value))?,
        )),
        _ => Err(invalid_duration(flag, value).into()),
    }
}

fn invalid_duration(flag: &str, value: &str) -> String {
    format!("invalid duration for {flag}: {value} (use forms like 6h, 2d, or 1w)")
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

fn validate_source_stores(config: &Config) -> AppResult<()> {
    let codex_exists = config.codex_home.is_dir();
    let claude_exists = config.claude_home.is_dir();
    match config.source {
        Source::Codex if !codex_exists => Err(format!(
            "Codex session store not found at {}; set --codex-home, CODEX_HOME, or AGENT_SESSION_FINDER_CODEX_HOME",
            config.codex_home.display()
        )
        .into()),
        Source::Claude if !claude_exists => Err(format!(
            "Claude session store not found at {}; set --claude-home, CLAUDE_CONFIG_DIR, or AGENT_SESSION_FINDER_CLAUDE_HOME",
            config.claude_home.display()
        )
        .into()),
        Source::All if !codex_exists && !claude_exists => Err(format!(
            "no session stores found (checked {} and {}); install Codex or Claude, or set a store path override",
            config.codex_home.display(),
            config.claude_home.display()
        )
        .into()),
        _ => Ok(()),
    }
}

fn open_db(path: &Path) -> AppResult<Connection> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "wal")?;
    conn.execute_batch(
        r#"
        create table if not exists sessions (
            session_id text primary key,
            title text not null,
            cwd text not null,
            updated_at text not null,
            source_path text not null,
            session_kind text not null default 'full',
            parent_thread_id text not null default '',
            repo text not null
        );
        create table if not exists docs (
            doc_id integer primary key,
            session_id text not null,
            source_kind text not null,
            source_path text not null,
            session_kind text not null default 'full',
            timestamp text not null,
            title text not null,
            cwd text not null,
            repo text not null,
            role text not null,
            category text not null,
            body text not null
        );
        create virtual table if not exists docs_fts using fts5(
            title,
            cwd,
            repo,
            role,
            category,
            body,
            source_path,
            tokenize='unicode61'
        );
        create table if not exists sources (
            source_path text primary key,
            mtime_ms integer not null,
            size_bytes integer not null
        );
        create table if not exists kv (
            key text primary key,
            value text not null
        );
        create index if not exists docs_session_id_idx on docs(session_id);
        "#,
    )?;
    ensure_column(
        &conn,
        "sessions",
        "session_kind",
        "text not null default 'unknown'",
    )?;
    ensure_column(
        &conn,
        "sessions",
        "parent_thread_id",
        "text not null default ''",
    )?;
    ensure_column(&conn, "sessions", "repo", "text not null default ''")?;
    ensure_column(&conn, "sessions", "created_at", "text not null default ''")?;
    ensure_column(&conn, "sessions", "archived", "integer not null default 0")?;
    ensure_column(
        &conn,
        "sessions",
        "first_user_message",
        "text not null default ''",
    )?;
    ensure_column(&conn, "sessions", "preview", "text not null default ''")?;
    ensure_column(
        &conn,
        "docs",
        "session_kind",
        "text not null default 'unknown'",
    )?;
    ensure_column(&conn, "docs", "repo", "text not null default ''")?;
    migrate_sources_schema(&conn)?;
    migrate_docs_fts_schema(&conn)?;
    ensure_index_version(&conn)?;
    Ok(conn)
}

fn ensure_column(conn: &Connection, table: &str, column: &str, definition: &str) -> AppResult<()> {
    if !column_exists(conn, table, column)? {
        conn.execute(
            &format!("alter table {table} add column {column} {definition}"),
            [],
        )?;
    }
    Ok(())
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> AppResult<bool> {
    let mut stmt = conn.prepare(&format!("pragma table_info({table})"))?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(names.iter().any(|name| name == column))
}

fn migrate_sources_schema(conn: &Connection) -> AppResult<()> {
    let has_mtime_ns = column_exists(conn, "sources", "mtime_ns")?;
    ensure_column(conn, "sources", "source_kind", "text not null default ''")?;
    ensure_column(conn, "sources", "mtime_ns", "integer not null default 0")?;
    ensure_column(conn, "sources", "mtime_ms", "integer not null default 0")?;
    ensure_column(conn, "sources", "indexed_at", "text not null default ''")?;
    if has_mtime_ns {
        conn.execute(
            "update sources set mtime_ms = mtime_ns / 1000000 where mtime_ms = 0",
            [],
        )?;
    }
    Ok(())
}

fn migrate_docs_fts_schema(conn: &Connection) -> AppResult<()> {
    if column_exists(conn, "docs_fts", "source_path")? && column_exists(conn, "docs_fts", "repo")? {
        return Ok(());
    }
    conn.execute_batch(
        r#"
        drop table if exists docs_fts;
        create virtual table docs_fts using fts5(
            title,
            cwd,
            repo,
            role,
            category,
            body,
            source_path,
            tokenize='unicode61'
        );
        delete from docs;
        delete from sessions;
        delete from sources;
        "#,
    )?;
    Ok(())
}

fn ensure_index_version(conn: &Connection) -> AppResult<()> {
    let current = conn
        .query_row(
            "select value from kv where key='rust_index_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if current.as_deref() != Some(INDEX_VERSION) {
        conn.execute_batch(
            r#"
            delete from docs_fts;
            delete from docs;
            delete from sessions;
            delete from sources;
            "#,
        )?;
        conn.execute(
            r#"
            insert into kv(key, value) values ('rust_index_version', ?1)
            on conflict(key) do update set value=excluded.value
            "#,
            [INDEX_VERSION],
        )?;
    }
    Ok(())
}

fn docs_empty(conn: &Connection) -> AppResult<bool> {
    let exists: i64 = conn.query_row("select exists(select 1 from docs limit 1)", [], |row| {
        row.get(0)
    })?;
    Ok(exists == 0)
}

fn source_scope_needs_auto_index(conn: &Connection, config: &Config) -> AppResult<bool> {
    if docs_empty(conn)? {
        return Ok(true);
    }
    if config.index_since.is_some() {
        return Ok(true);
    }
    match config.source {
        Source::All => {
            let codex_sources = find_codex_sources(&config.codex_home, !config.no_archived)?;
            if !codex_sources.is_empty()
                && source_docs_empty(conn, Source::Codex, config.no_archived)?
            {
                return Ok(true);
            }
            let claude_sources = find_claude_sources(&config.claude_home)?;
            if !claude_sources.is_empty()
                && source_docs_empty(conn, Source::Claude, config.no_archived)?
            {
                return Ok(true);
            }
            Ok(false)
        }
        source => source_docs_empty(conn, source, config.no_archived),
    }
}

fn source_docs_empty(conn: &Connection, source: Source, no_archived: bool) -> AppResult<bool> {
    let prefix = source_prefix(source);
    if prefix.is_empty() {
        return docs_empty(conn);
    }
    let exists: i64 = conn.query_row(
        r#"
        select exists(
            select 1 from docs
            where source_kind like ?1 || '%'
              and (?2 = 0 or source_path not like '%/archived_sessions/%')
            limit 1
        )
        "#,
        params![prefix, i64::from(no_archived)],
        |row| row.get(0),
    )?;
    Ok(exists == 0)
}

fn index_all(conn: &mut Connection, config: &Config) -> AppResult<IndexSummary> {
    let mut indexed = 0;
    let mut skipped = 0;
    let mut filtered = 0;

    let mut sources = Vec::new();
    let mut discovered_sources = Vec::new();
    if matches!(config.source, Source::All | Source::Codex) {
        let codex_sources = find_codex_sources(&config.codex_home, !config.no_archived)?;
        discovered_sources.extend(codex_sources.iter().cloned());
        if config.no_archived {
            sources.extend(
                codex_sources
                    .into_iter()
                    .filter(|source| !is_archived_source_path(source)),
            );
        } else {
            sources.extend(codex_sources);
        }
    }
    if matches!(config.source, Source::All | Source::Claude) {
        let claude_sources = find_claude_sources(&config.claude_home)?;
        discovered_sources.extend(claude_sources.iter().cloned());
        sources.extend(claude_sources);
    }
    sources.sort_by(|left, right| {
        source_recency_key(right)
            .cmp(&source_recency_key(left))
            .then_with(|| right.cmp(left))
    });
    let discovered_sources = discovered_sources
        .iter()
        .map(|source| source.display().to_string())
        .collect::<HashSet<_>>();
    let sources = select_index_sources(sources, config)?;

    let tx = conn.transaction()?;
    reconcile_missing_sources(&tx, config.source, config.no_archived, &discovered_sources)?;
    for source in sources {
        if let Some(duration) = config.index_since {
            if !source_is_recent(&source, duration) {
                continue;
            }
        }
        if !config.include_workers && !is_claude_source(&source, &config.claude_home) {
            if let Some((_, _, session_kind)) = codex_thread_info(&source)? {
                if session_kind == "subagent" {
                    delete_source_docs(&tx, &source)?;
                    mark_source(&tx, &source, "codex-jsonl")?;
                    filtered += 1;
                    continue;
                }
            }
        }
        if source_unchanged(&tx, &source, config.include_workers)? {
            skipped += 1;
            continue;
        }
        if let Some(max) = config.max_sources {
            if indexed >= max {
                break;
            }
        }
        delete_source_docs(&tx, &source)?;
        let source_kind = if is_claude_source(&source, &config.claude_home) {
            "claude-jsonl"
        } else {
            "codex-jsonl"
        };
        let (docs, meta) = if is_claude_source(&source, &config.claude_home) {
            parse_claude_file(&source)?
        } else {
            parse_codex_file(&source)?
        };
        if !config.include_workers
            && meta
                .as_ref()
                .map(|meta| meta.session_kind == "subagent")
                .unwrap_or(false)
        {
            mark_source(&tx, &source, source_kind)?;
            filtered += 1;
            continue;
        }
        if let Some(meta) = meta {
            upsert_session(&tx, &meta)?;
        }
        for doc in docs {
            insert_doc(&tx, &doc)?;
        }
        mark_source(&tx, &source, source_kind)?;
        indexed += 1;
    }
    tx.commit()?;
    let docs: i64 = conn.query_row("select count(*) from docs", [], |row| row.get(0))?;
    Ok(IndexSummary {
        indexed,
        skipped,
        filtered,
        docs,
    })
}

fn reconcile_missing_sources(
    conn: &Connection,
    source: Source,
    no_archived: bool,
    discovered_sources: &HashSet<String>,
) -> AppResult<usize> {
    let mut stmt = conn.prepare("select source_path, source_kind from sources")?;
    let indexed_sources = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut removed = 0;
    for (source_path, source_kind) in indexed_sources {
        if !source_matches_filter(source, &source_path, &source_kind)
            || (no_archived && is_archived_source_path(Path::new(&source_path)))
            || discovered_sources.contains(&source_path)
        {
            continue;
        }
        delete_source_docs(conn, Path::new(&source_path))?;
        conn.execute("delete from sources where source_path=?1", [&source_path])?;
        removed += 1;
    }
    Ok(removed)
}

fn source_matches_filter(source: Source, source_path: &str, source_kind: &str) -> bool {
    match source {
        Source::All => true,
        Source::Codex => {
            source_kind.starts_with("codex")
                || (source_kind.is_empty()
                    && !source_path.contains("/.claude/")
                    && !source_path.contains("/transcripts/"))
        }
        Source::Claude => {
            source_kind.starts_with("claude")
                || (source_kind.is_empty()
                    && (source_path.contains("/.claude/") || source_path.contains("/transcripts/")))
        }
    }
}

fn find_codex_sources(codex_home: &Path, include_archived: bool) -> AppResult<Vec<PathBuf>> {
    let mut paths = Vec::new();
    collect_jsonl(&codex_home.join("sessions"), &mut paths)?;
    if include_archived {
        collect_jsonl(&codex_home.join("archived_sessions"), &mut paths)?;
    }
    paths.sort();
    paths.reverse();
    Ok(paths)
}

fn is_archived_source_path(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "archived_sessions")
}

fn find_claude_sources(claude_home: &Path) -> AppResult<Vec<PathBuf>> {
    let mut paths = Vec::new();
    collect_jsonl(&claude_home.join("transcripts"), &mut paths)?;
    collect_claude_project_jsonl(&claude_home.join("projects"), &mut paths)?;
    paths.sort();
    paths.reverse();
    Ok(paths)
}

fn select_index_sources(sources: Vec<PathBuf>, config: &Config) -> AppResult<Vec<PathBuf>> {
    let mut selected = Vec::new();
    for source in &sources {
        if let Some(duration) = config.index_since {
            if !source_is_recent(source, duration) {
                continue;
            }
        }
        selected.push(source.clone());
    }
    Ok(selected)
}

fn codex_thread_info(path: &Path) -> AppResult<Option<(String, String, String)>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    for line in reader.lines().take(20) {
        let line = line?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if text_at(&value, &["type"]) != "session_meta" {
            continue;
        }
        let payload = &value["payload"];
        let thread_id = text_at(payload, &["id"]);
        let parent_id = text_at(payload, &["parent_thread_id"]);
        let session_kind = if is_subagent_session_meta(payload, &parent_id) {
            "subagent"
        } else {
            "full"
        };
        return Ok(Some((thread_id, parent_id, session_kind.to_string())));
    }
    Ok(None)
}

fn is_subagent_session_meta(payload: &Value, parent_thread_id: &str) -> bool {
    !parent_thread_id.is_empty()
        || text_at(payload, &["thread_source"]).eq_ignore_ascii_case("subagent")
        || !payload["source"]["subagent"].is_null()
}

fn collect_jsonl(root: &Path, paths: &mut Vec<PathBuf>) -> AppResult<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl(&path, paths)?;
        } else if path.extension().and_then(|value| value.to_str()) == Some("jsonl") {
            paths.push(path);
        }
    }
    Ok(())
}

fn collect_claude_project_jsonl(root: &Path, paths: &mut Vec<PathBuf>) -> AppResult<()> {
    if !root.exists() {
        return Ok(());
    }
    for project in fs::read_dir(root)? {
        let project = project?;
        let project_path = project.path();
        if !project_path.is_dir() {
            continue;
        }
        for entry in fs::read_dir(project_path)? {
            let path = entry?.path();
            if path.is_file() && path.extension().and_then(|value| value.to_str()) == Some("jsonl")
            {
                paths.push(path);
            }
        }
    }
    Ok(())
}

fn is_claude_source(path: &Path, claude_home: &Path) -> bool {
    path.starts_with(claude_home)
}

fn parse_codex_file(path: &Path) -> AppResult<(Vec<RawDoc>, Option<SessionMeta>)> {
    let mut session_id = session_id_from_path(path);
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut raw_docs = Vec::new();
    let mut cwd = String::new();
    let mut title = String::new();
    let mut updated_at = String::new();
    let mut session_kind = "full".to_string();
    let mut parent_thread_id = String::new();
    let mut last_user_prompt = String::new();

    for line in reader.lines() {
        let line = line?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let timestamp = text_at(&value, &["timestamp"]);
        if !timestamp.is_empty() {
            updated_at = timestamp.clone();
        }
        let row_type = text_at(&value, &["type"]);
        let payload = &value["payload"];
        if row_type == "session_meta" {
            let found_id = text_at(payload, &["id"]);
            if !found_id.is_empty() {
                session_id = found_id;
            }
            let found_cwd = text_at(payload, &["cwd"]);
            if !found_cwd.is_empty() {
                cwd = found_cwd;
            }
            let found_parent = text_at(payload, &["parent_thread_id"]);
            if !found_parent.is_empty() {
                parent_thread_id = found_parent;
            }
            if is_subagent_session_meta(payload, &parent_thread_id) {
                session_kind = "subagent".to_string();
            }
            continue;
        }
        if row_type == "turn_context" {
            let found_cwd = text_at(payload, &["cwd"]);
            if !found_cwd.is_empty() {
                cwd = found_cwd;
            }
            continue;
        }
        let mut maybe_doc = None;
        if row_type == "event_msg" {
            match text_at(payload, &["type"]).as_str() {
                "user_message" => {
                    let message = text_at(payload, &["message"]);
                    maybe_doc = compact_subagent_notification(&message)
                        .map(|summary| ("assistant", "worker_summary", summary))
                        .or(Some(("user", "user_prompt", message)));
                }
                "agent_message" => {
                    maybe_doc = Some((
                        "assistant",
                        "assistant_message",
                        text_at(payload, &["message"]),
                    ))
                }
                "exec_command_end" => {
                    let exit_code = event_msg_exit_code(payload);
                    let output = event_msg_tool_output(payload);
                    let failed = exit_code.map(|code| code != 0).unwrap_or(false);
                    if exit_code != Some(0) && (failed || looks_like_error(&output)) {
                        maybe_doc = Some(("tool", "error_text", trim(&output, ERROR_CHARS)));
                    }
                }
                _ => {}
            }
        } else if row_type == "response_item" {
            match text_at(payload, &["type"]).as_str() {
                "message" => {
                    let role = text_at(payload, &["role"]);
                    if role != "developer" && role != "system" {
                        maybe_doc = Some((
                            if role == "user" { "user" } else { "assistant" },
                            if role == "user" {
                                "user_prompt"
                            } else {
                                "assistant_message"
                            },
                            content_text(&payload["content"]),
                        ));
                    }
                }
                "function_call" => {
                    maybe_doc = Some(("tool", "tool_command", compact_function_call(payload)));
                }
                "function_call_output" => {
                    let output = content_text(&payload["output"]);
                    if looks_like_error(&output) {
                        maybe_doc = Some(("tool", "error_text", trim(&output, ERROR_CHARS)));
                    }
                }
                "custom_tool_call" | "tool_search_call" | "web_search_call" => {
                    maybe_doc = Some(("tool", "tool_command", compact_custom_tool_call(payload)));
                }
                "custom_tool_call_output" | "tool_search_output" => {
                    let output = custom_tool_output_text(payload);
                    if looks_like_error(&output) {
                        maybe_doc = Some(("tool", "error_text", trim(&output, ERROR_CHARS)));
                    }
                }
                _ => {}
            }
        }
        if let Some((role, category, body)) = maybe_doc {
            let body = if category == "user_prompt" {
                normalize_user_text(&body)
            } else {
                body
            };
            if body.trim().is_empty() || should_skip_user_text(&body) {
                continue;
            }
            if category == "user_prompt" {
                if body == last_user_prompt {
                    continue;
                }
                last_user_prompt = body.clone();
            } else {
                last_user_prompt.clear();
            }
            if title.is_empty() && category == "user_prompt" {
                title = first_line(&body);
            }
            raw_docs.push(RawDoc {
                session_id: session_id.clone(),
                source_kind: "codex-jsonl".to_string(),
                source_path: path.display().to_string(),
                session_kind: session_kind.clone(),
                timestamp,
                title: title.clone(),
                cwd: cwd.clone(),
                repo: repo_name(&cwd),
                role: role.to_string(),
                category: category.to_string(),
                body,
            });
        }
    }

    if title.is_empty() {
        title = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
    }
    for doc in &mut raw_docs {
        doc.session_id = session_id.clone();
        doc.session_kind = session_kind.clone();
    }
    let meta = SessionMeta {
        session_id: session_id.clone(),
        title: title.clone(),
        cwd: cwd.clone(),
        updated_at: updated_at.clone(),
        source_path: path.display().to_string(),
        session_kind: session_kind.clone(),
        parent_thread_id: parent_thread_id.clone(),
        repo: repo_name(&cwd),
    };
    let mut docs = vec![RawDoc {
        session_id,
        source_kind: "codex-metadata".to_string(),
        source_path: path.display().to_string(),
        session_kind: session_kind.clone(),
        timestamp: updated_at,
        title: title.clone(),
        cwd: cwd.clone(),
        repo: repo_name(&cwd),
        role: "metadata".to_string(),
        category: "metadata".to_string(),
        body: format!("{title}\n{cwd}\nsession: {session_kind}\nparent: {parent_thread_id}"),
    }];
    docs.extend(compact_windows(raw_docs));
    Ok((docs, Some(meta)))
}

fn parse_claude_file(path: &Path) -> AppResult<(Vec<RawDoc>, Option<SessionMeta>)> {
    let session_id = format!(
        "claude:{}",
        path.file_stem().unwrap_or_default().to_string_lossy()
    );
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut raw_docs = Vec::new();
    let mut title = String::new();
    let mut updated_at = String::new();
    let session_kind = "full".to_string();
    let mut cwd = String::new();
    for line in reader.lines() {
        let line = line?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value["isMeta"].as_bool().unwrap_or(false) {
            continue;
        }
        if value["isSidechain"].as_bool().unwrap_or(false) {
            continue;
        }
        let timestamp = text_at(&value, &["timestamp"]);
        if !timestamp.is_empty() {
            updated_at = timestamp.clone();
        }
        let row_cwd = claude_row_cwd(&value);
        if cwd.is_empty() && !row_cwd.is_empty() {
            cwd = row_cwd.clone();
        }
        for (role, category, body) in claude_nested_tool_docs(&value) {
            raw_docs.push(RawDoc {
                session_id: session_id.clone(),
                source_kind: "claude-jsonl".to_string(),
                source_path: path.display().to_string(),
                session_kind: session_kind.clone(),
                timestamp: timestamp.clone(),
                title: title.clone(),
                cwd: row_cwd.clone(),
                repo: repo_name(&row_cwd),
                role: role.to_string(),
                category: category.to_string(),
                body,
            });
        }
        let row_type = text_at(&value, &["type"]);
        let (role, category, body) = match row_type.as_str() {
            "user" => ("user", "user_prompt", claude_message_text(&value)),
            "assistant" => (
                "assistant",
                "assistant_message",
                claude_message_text(&value),
            ),
            "tool_use" => ("tool", "tool_command", compact_claude_tool(&value)),
            "tool_result" => {
                let output = claude_tool_output(&value);
                if looks_like_error(&output) {
                    ("tool", "error_text", trim(&output, ERROR_CHARS))
                } else {
                    continue;
                }
            }
            "" if path.file_name().and_then(|name| name.to_str()) == Some("history.jsonl") => {
                ("user", "user_prompt", text_at(&value, &["display"]))
            }
            _ => continue,
        };
        let body = if category == "user_prompt" {
            normalize_user_text(&body)
        } else {
            body
        };
        if category == "user_prompt" && should_skip_user_text(&body) {
            continue;
        }
        if body.trim().is_empty() {
            continue;
        }
        if title.is_empty() && category == "user_prompt" {
            title = first_line(&body);
        }
        raw_docs.push(RawDoc {
            session_id: session_id.clone(),
            source_kind: "claude-jsonl".to_string(),
            source_path: path.display().to_string(),
            session_kind: session_kind.clone(),
            timestamp,
            title: title.clone(),
            cwd: row_cwd.clone(),
            repo: repo_name(&row_cwd),
            role: role.to_string(),
            category: category.to_string(),
            body,
        });
    }
    if raw_docs.is_empty() {
        return Ok((Vec::new(), None));
    }
    if title.is_empty() {
        title = format!("Claude transcript {}", path.display());
    }
    for doc in &mut raw_docs {
        doc.session_kind = session_kind.clone();
    }
    let meta = SessionMeta {
        session_id: session_id.clone(),
        title: title.clone(),
        cwd: cwd.clone(),
        updated_at: updated_at.clone(),
        source_path: path.display().to_string(),
        session_kind: session_kind.clone(),
        parent_thread_id: String::new(),
        repo: repo_name(&cwd),
    };
    let mut docs = vec![RawDoc {
        session_id,
        source_kind: "claude-metadata".to_string(),
        source_path: path.display().to_string(),
        session_kind,
        timestamp: updated_at,
        title: title.clone(),
        cwd: cwd.clone(),
        repo: repo_name(&cwd),
        role: "metadata".to_string(),
        category: "metadata".to_string(),
        body: format!("{title}\n{cwd}"),
    }];
    docs.extend(compact_windows(raw_docs));
    Ok((docs, Some(meta)))
}

fn compact_windows(raw_docs: Vec<RawDoc>) -> Vec<RawDoc> {
    let mut compacted = Vec::new();
    let mut standalone_errors = 0;
    for (index, doc) in raw_docs.iter().enumerate() {
        if doc.category == "error_text" {
            if standalone_errors < MAX_STANDALONE_ERRORS {
                compacted.push(doc.clone());
                standalone_errors += 1;
            }
            continue;
        }
        if doc.category != "user_prompt" {
            continue;
        }
        let mut body = format!("User: {}", trim(&doc.body, 2_000));
        let mut count = 0;
        for next in raw_docs.iter().skip(index + 1) {
            if next.category == "user_prompt" {
                break;
            }
            if next.category == "tool_output" {
                continue;
            }
            count += 1;
            if count > CONTEXT_DOCS {
                break;
            }
            body.push_str("\n\n");
            body.push_str(match next.category.as_str() {
                "assistant_message" => "Assistant: ",
                "tool_command" => "Tool: ",
                "error_text" => "Error: ",
                _ => "Context: ",
            });
            body.push_str(&trim(&next.body, 1_200));
        }
        let mut window = doc.clone();
        window.role = "window".to_string();
        window.category = "conversation_window".to_string();
        window.body = trim(&body, MAX_BODY_CHARS);
        compacted.push(window);
    }
    compacted
}

fn upsert_session(conn: &Connection, meta: &SessionMeta) -> AppResult<()> {
    conn.execute(
        r#"
        insert into sessions(session_id, title, cwd, updated_at, source_path, session_kind, parent_thread_id, repo)
        values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        on conflict(session_id) do update set
            title=excluded.title,
            cwd=excluded.cwd,
            updated_at=excluded.updated_at,
            source_path=excluded.source_path,
            session_kind=excluded.session_kind,
            parent_thread_id=excluded.parent_thread_id,
            repo=excluded.repo
        "#,
        params![
            meta.session_id,
            meta.title,
            meta.cwd,
            meta.updated_at,
            meta.source_path,
            meta.session_kind,
            meta.parent_thread_id,
            meta.repo
        ],
    )?;
    Ok(())
}

fn insert_doc(conn: &Connection, doc: &RawDoc) -> AppResult<()> {
    conn.execute(
        r#"
        insert into docs(session_id, source_kind, source_path, session_kind, timestamp, title, cwd, repo, role, category, body)
        values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
        params![
            doc.session_id,
            doc.source_kind,
            doc.source_path,
            doc.session_kind,
            doc.timestamp,
            doc.title,
            doc.cwd,
            doc.repo,
            doc.role,
            doc.category,
            doc.body
        ],
    )?;
    let rowid = conn.last_insert_rowid();
    conn.execute(
        r#"
        insert into docs_fts(rowid, title, cwd, repo, role, category, body, source_path)
        values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
        params![
            rowid,
            doc.title,
            doc.cwd,
            doc.repo,
            doc.role,
            doc.category,
            doc.body,
            doc.source_path
        ],
    )?;
    Ok(())
}

fn delete_source_docs(conn: &Connection, path: &Path) -> AppResult<()> {
    let source = path.display().to_string();
    let mut stmt = conn.prepare("select doc_id, session_id from docs where source_path=?1")?;
    let rows = stmt
        .query_map([&source], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let session_ids = rows
        .iter()
        .map(|(_, session_id)| session_id.clone())
        .collect::<BTreeSet<_>>();
    for (id, _) in rows {
        conn.execute("delete from docs_fts where rowid=?1", [id])?;
    }
    conn.execute("delete from docs where source_path=?1", [&source])?;
    for session_id in session_ids {
        let remaining: i64 = conn.query_row(
            "select exists(select 1 from docs where session_id=?1 limit 1)",
            [&session_id],
            |row| row.get(0),
        )?;
        if remaining == 0 {
            conn.execute("delete from sessions where session_id=?1", [&session_id])?;
        }
    }
    Ok(())
}

fn source_unchanged(conn: &Connection, path: &Path, include_workers: bool) -> AppResult<bool> {
    let source = path.display().to_string();
    let Some((stored_mtime_ns, stored_mtime_ms, stored_size)) = conn
        .query_row(
            "select mtime_ns, mtime_ms, size_bytes from sources where source_path=?1",
            [&source],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok(false);
    };
    let metadata = fs::metadata(path)?;
    let same_mtime = if stored_mtime_ns > 0 {
        stored_mtime_ns == mtime_ns(&metadata)?
    } else {
        stored_mtime_ms == mtime_ms(&metadata)?
    };
    if !same_mtime || stored_size != metadata.len() as i64 {
        return Ok(false);
    }
    if include_workers {
        let source = path.display().to_string();
        let has_docs: i64 = conn.query_row(
            "select exists(select 1 from docs where source_path=?1 limit 1)",
            [&source],
            |row| row.get(0),
        )?;
        if has_docs == 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

fn mark_source(conn: &Connection, path: &Path, source_kind: &str) -> AppResult<()> {
    let metadata = fs::metadata(path)?;
    let modified = metadata.modified()?.duration_since(UNIX_EPOCH)?;
    let mtime_ns = modified.as_nanos().min(i64::MAX as u128) as i64;
    let mtime_ms = modified.as_millis() as i64;
    let indexed_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs()
        .to_string();
    conn.execute(
        r#"
        insert into sources(source_path, source_kind, mtime_ns, mtime_ms, size_bytes, indexed_at)
        values (?1, ?2, ?3, ?4, ?5, ?6)
        on conflict(source_path) do update set
            source_kind=excluded.source_kind,
            mtime_ns=excluded.mtime_ns,
            mtime_ms=excluded.mtime_ms,
            size_bytes=excluded.size_bytes,
            indexed_at=excluded.indexed_at
        "#,
        params![
            path.display().to_string(),
            source_kind,
            mtime_ns,
            mtime_ms,
            metadata.len() as i64,
            indexed_at
        ],
    )?;
    Ok(())
}

fn search(conn: &Connection, config: &Config) -> AppResult<Vec<ResultCard>> {
    let terms = query_terms(&config.query);
    if terms.is_empty() {
        if config.cwd.as_deref().unwrap_or_default().is_empty() {
            return Ok(Vec::new());
        }
        return search_metadata(conn, config, &terms);
    }
    let fts = fts_query(&terms);
    let source_prefix = source_prefix(config.source);
    let cwd = config.cwd.as_deref().unwrap_or_default();
    let since_seconds = search_since_seconds(config);
    let exclude_archived = i64::from(config.no_archived);
    let include_workers = i64::from(config.include_workers);
    let mut stmt = conn.prepare(
        r#"
        select d.session_id,
               coalesce(nullif(s.title,''), d.title) as title,
               coalesce(nullif(s.cwd,''), d.cwd) as cwd,
               coalesce(nullif(s.updated_at,''), d.timestamp) as updated_at,
               coalesce(nullif(s.source_path,''), d.source_path) as source_path,
               coalesce(nullif(s.session_kind,''), d.session_kind) as session_kind,
               coalesce(nullif(s.parent_thread_id,''), '') as parent_thread_id,
               d.category,
               d.body
        from docs_fts
        join docs d on d.doc_id = docs_fts.rowid
        left join sessions s on s.session_id = d.session_id
        where docs_fts match ?1
          and (?2 = '' or d.source_kind like ?2 || '%')
          and (?3 = ''
               or instr(coalesce(nullif(s.cwd,''), d.cwd), ?3) > 0
               or instr(coalesce(nullif(s.repo,''), d.repo), ?3) > 0
               or instr(d.source_path, ?3) > 0)
          and (?4 = 0 or unixepoch(coalesce(nullif(s.updated_at,''), d.timestamp)) >= unixepoch('now') - ?4)
          and (?5 = 0 or d.source_path not like '%/archived_sessions/%')
          and (?6 = 1 or coalesce(nullif(s.session_kind,''), d.session_kind) != 'subagent')
        order by bm25(docs_fts), coalesce(nullif(s.updated_at,''), d.timestamp) desc
        limit 1200
        "#,
    )?;
    let rows = stmt
        .query_map(
            params![
                fts,
                source_prefix,
                cwd,
                since_seconds,
                exclude_archived,
                include_workers
            ],
            |row| {
                Ok(SearchRow {
                    session_id: row.get(0)?,
                    title: row.get(1)?,
                    cwd: row.get(2)?,
                    updated_at: row.get(3)?,
                    source_path: row.get(4)?,
                    session_kind: row.get(5)?,
                    parent_thread_id: row.get(6)?,
                    category: row.get(7)?,
                    body: row.get(8)?,
                    rank: 0.0,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(result_cards_from_rows(rows, &terms, config))
}

fn search_metadata(
    conn: &Connection,
    config: &Config,
    terms: &[String],
) -> AppResult<Vec<ResultCard>> {
    let source_prefix = source_prefix(config.source);
    let cwd = config.cwd.as_deref().unwrap_or_default();
    let since_seconds = search_since_seconds(config);
    let exclude_archived = i64::from(config.no_archived);
    let include_workers = i64::from(config.include_workers);
    let mut stmt = conn.prepare(
        r#"
        select d.session_id,
               coalesce(nullif(s.title,''), d.title) as title,
               coalesce(nullif(s.cwd,''), d.cwd) as cwd,
               coalesce(nullif(s.updated_at,''), d.timestamp) as updated_at,
               coalesce(nullif(s.source_path,''), d.source_path) as source_path,
               coalesce(nullif(s.session_kind,''), d.session_kind) as session_kind,
               coalesce(nullif(s.parent_thread_id,''), '') as parent_thread_id,
               d.category,
               d.body
        from docs d
        left join sessions s on s.session_id = d.session_id
        where d.category = 'metadata'
          and (?1 = '' or d.source_kind like ?1 || '%')
          and (?2 = ''
               or instr(coalesce(nullif(s.cwd,''), d.cwd), ?2) > 0
               or instr(coalesce(nullif(s.repo,''), d.repo), ?2) > 0
               or instr(d.source_path, ?2) > 0)
          and (?3 = 0 or unixepoch(coalesce(nullif(s.updated_at,''), d.timestamp)) >= unixepoch('now') - ?3)
          and (?4 = 0 or d.source_path not like '%/archived_sessions/%')
          and (?5 = 1 or coalesce(nullif(s.session_kind,''), d.session_kind) != 'subagent')
        order by coalesce(nullif(s.updated_at,''), d.timestamp) desc
        limit 1200
        "#,
    )?;
    let rows = stmt
        .query_map(
            params![
                source_prefix,
                cwd,
                since_seconds,
                exclude_archived,
                include_workers
            ],
            |row| {
                Ok(SearchRow {
                    session_id: row.get(0)?,
                    title: row.get(1)?,
                    cwd: row.get(2)?,
                    updated_at: row.get(3)?,
                    source_path: row.get(4)?,
                    session_kind: row.get(5)?,
                    parent_thread_id: row.get(6)?,
                    category: row.get(7)?,
                    body: row.get(8)?,
                    rank: 0.0,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(result_cards_from_rows(rows, terms, config))
}

fn source_prefix(source: Source) -> &'static str {
    match source {
        Source::All => "",
        Source::Codex => "codex",
        Source::Claude => "claude",
    }
}

fn search_since_seconds(config: &Config) -> i64 {
    config
        .since
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0)
}

fn result_cards_from_rows(
    rows: Vec<SearchRow>,
    terms: &[String],
    config: &Config,
) -> Vec<ResultCard> {
    let mut grouped: BTreeMap<String, ResultCard> = BTreeMap::new();
    for mut row in rows {
        let matched = matched_terms(&row, terms);
        row.rank = row_rank(&row, terms, matched.len());
        let entry = grouped
            .entry(row.session_id.clone())
            .or_insert_with(|| ResultCard {
                session_id: row.session_id.clone(),
                title: row.title.clone(),
                cwd: row.cwd.clone(),
                updated_at: row.updated_at.clone(),
                source_path: row.source_path.clone(),
                session_kind: normalized_session_kind(&row.session_kind, &row.title, &row.body),
                parent_thread_id: row.parent_thread_id.clone(),
                categories: BTreeSet::new(),
                snippets: Vec::new(),
                matched_terms: BTreeSet::new(),
                best_row_terms: 0,
                rank: 0.0,
            });
        entry.rank = entry.rank.max(row.rank);
        if session_kind_priority(&row.session_kind, &row.title, &row.body)
            > session_kind_priority(&entry.session_kind, &entry.title, "")
        {
            entry.session_kind = normalized_session_kind(&row.session_kind, &row.title, &row.body);
        }
        if entry.parent_thread_id.is_empty() && !row.parent_thread_id.is_empty() {
            entry.parent_thread_id = row.parent_thread_id.clone();
        }
        entry.best_row_terms = entry.best_row_terms.max(matched.len());
        entry.categories.insert(row.category.clone());
        entry.matched_terms.extend(matched);
        if entry.snippets.len() < 3 {
            entry.snippets.push(snippet(&row.body, terms));
        }
    }
    let mut results: Vec<_> = grouped.into_values().collect();
    if !config.include_workers {
        results.retain(|result| result.session_kind != "subagent");
    }
    results.sort_by(|a, b| {
        b.matched_terms
            .len()
            .cmp(&a.matched_terms.len())
            .then_with(|| b.best_row_terms.cmp(&a.best_row_terms))
            .then_with(|| session_kind_sort_value(b).cmp(&session_kind_sort_value(a)))
            .then_with(|| {
                b.rank
                    .partial_cmp(&a.rank)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| b.updated_at.cmp(&a.updated_at))
    });
    results.retain(|result| {
        result.best_row_terms >= terms.len().min(2)
            || result.matched_terms.len() >= terms.len().saturating_sub(1).max(1)
    });
    results.truncate(config.limit);
    results
}

fn print_results(results: &[ResultCard], query: &str, limit: usize) -> AppResult<()> {
    let total_terms = query_terms(query).len().max(1);
    let mut stdout = io::stdout().lock();
    for (index, result) in results.iter().take(limit).enumerate() {
        write_io(writeln!(
            stdout,
            "{}. {}",
            index + 1,
            one_line(&result.title, 92)
        ))?;
        let categories = result
            .categories
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(",");
        write_io(writeln!(
            stdout,
            "   match: {}/{}  kind: {}  score: {:.1}",
            result.matched_terms.len(),
            total_terms,
            categories,
            result.rank
        ))?;
        write_io(writeln!(stdout, "   session: {}", session_display(result)))?;
        if result.session_kind == "subagent" {
            if !result.parent_thread_id.is_empty() {
                write_io(writeln!(stdout, "   parent: {}", result.parent_thread_id))?;
            }
            write_io(writeln!(
                stdout,
                "   note: worker transcript JSONL; not a normal sidebar thread"
            ))?;
        }
        let terms = result
            .matched_terms
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(",");
        write_io(writeln!(stdout, "   terms: {terms}"))?;
        write_io(writeln!(stdout, "   cwd: {}", one_line(&result.cwd, 96)))?;
        write_io(writeln!(stdout, "   id:  {}", result.session_id))?;
        write_io(writeln!(stdout, "   src: {}", result.source_path))?;
        for hit in result.snippets.iter().take(2) {
            write_io(writeln!(stdout, "   hit: {}", one_line(hit, 140)))?;
        }
        write_io(writeln!(stdout))?;
    }
    Ok(())
}

fn session_display(result: &ResultCard) -> String {
    if result.session_kind == "subagent" {
        "worker/subagent".to_string()
    } else {
        result.session_kind.clone()
    }
}

fn write_io<T>(result: io::Result<T>) -> AppResult<T> {
    match result {
        Ok(value) => Ok(value),
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => std::process::exit(0),
        Err(err) => Err(err.into()),
    }
}

fn fts_query(terms: &[String]) -> String {
    let mut words = Vec::new();
    for term in terms {
        for variant in variants(term) {
            if !words.contains(&variant) {
                words.push(variant);
            }
        }
    }
    words
        .iter()
        .map(|word| format!("\"{}\"", word.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn query_terms(query: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for piece in query.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-') {
        let piece = piece.trim().to_lowercase();
        if piece.is_empty() || is_stopword(&piece) || terms.contains(&piece) {
            continue;
        }
        terms.push(piece);
    }
    terms
}

fn is_stopword(term: &str) -> bool {
    matches!(
        term,
        "a" | "an"
            | "and"
            | "are"
            | "as"
            | "at"
            | "be"
            | "but"
            | "by"
            | "can"
            | "do"
            | "doing"
            | "for"
            | "from"
            | "have"
            | "i"
            | "in"
            | "into"
            | "is"
            | "it"
            | "me"
            | "my"
            | "of"
            | "on"
            | "or"
            | "our"
            | "please"
            | "the"
            | "this"
            | "to"
            | "we"
            | "with"
            | "you"
            | "your"
    )
}

fn variants(term: &str) -> Vec<String> {
    match term {
        "bug" | "bugs" | "issue" | "issues" | "regression" | "regressions" => [
            "bug",
            "bugs",
            "issue",
            "issues",
            "regression",
            "regressions",
            "failure",
            "failures",
            "error",
            "errors",
        ]
        .iter()
        .map(|value| value.to_string())
        .collect(),
        "ui" => ["ui", "screen", "view", "page", "frontend", "interface"]
            .iter()
            .map(|value| value.to_string())
            .collect(),
        _ => vec![term.to_string()],
    }
}

fn normalized_session_kind(kind: &str, _title: &str, _body: &str) -> String {
    let kind = kind.trim().to_lowercase();
    if kind == "subagent" {
        "subagent".to_string()
    } else if kind == "full" || kind == "human" || kind.is_empty() || kind == "unknown" {
        "full".to_string()
    } else {
        kind
    }
}

fn session_kind_priority(kind: &str, title: &str, body: &str) -> usize {
    match normalized_session_kind(kind, title, body).as_str() {
        "subagent" => 0,
        _ => 1,
    }
}

fn session_kind_sort_value(result: &ResultCard) -> usize {
    match result.session_kind.as_str() {
        "subagent" => 0,
        _ => 1,
    }
}

fn matched_terms(row: &SearchRow, terms: &[String]) -> Vec<String> {
    let haystack = format!(
        "{} {} {} {} {}",
        row.title, row.cwd, row.category, row.body, row.source_path
    )
    .to_lowercase();
    terms
        .iter()
        .filter(|term| {
            variants(term)
                .iter()
                .any(|variant| contains_variant(&haystack, variant))
        })
        .cloned()
        .collect()
}

fn row_rank(row: &SearchRow, terms: &[String], matched_count: usize) -> f64 {
    let category_weight = match row.category.as_str() {
        "metadata" | "conversation_window" | "user_prompt" | "worker_summary" => 4.0,
        "assistant_message" => 2.0,
        "tool_command" => 1.0,
        "error_text" => 0.7,
        _ => 0.4,
    };
    let mut rank = matched_count as f64 * category_weight;
    let title = row.title.to_lowercase();
    for term in terms {
        if variants(term)
            .iter()
            .any(|variant| contains_variant(&title, variant))
        {
            rank += 3.0;
        }
    }
    let lower = row.body.to_lowercase();
    if lower.contains("export const") || lower.contains("export type") {
        rank -= 2.0;
    }
    rank
}

fn snippet(body: &str, terms: &[String]) -> String {
    let lower = body.to_lowercase();
    let mut best = 0;
    for term in terms {
        for variant in variants(term) {
            if let Some(index) = find_variant_index(&lower, &variant) {
                best = previous_word_boundary(body, index.saturating_sub(50));
                break;
            }
        }
    }
    trim(safe_slice(body, best, best + 220), 220)
}

fn contains_variant(haystack: &str, variant: &str) -> bool {
    if token_parts(variant).len() > 1 {
        find_token_sequence(haystack, variant).is_some()
    } else if variant.chars().count() <= 3 {
        find_short_token(haystack, variant).is_some()
    } else {
        haystack.contains(variant)
    }
}

fn find_variant_index(haystack: &str, variant: &str) -> Option<usize> {
    if token_parts(variant).len() > 1 {
        find_token_sequence(haystack, variant)
    } else if variant.chars().count() <= 3 {
        find_short_token(haystack, variant)
    } else {
        haystack.find(variant)
    }
}

fn token_parts(text: &str) -> Vec<&str> {
    text.split(|ch: char| !is_token_char(ch))
        .filter(|part| !part.is_empty())
        .collect()
}

fn find_token_sequence(haystack: &str, needle: &str) -> Option<usize> {
    let parts = token_parts(needle);
    if parts.len() <= 1 {
        return find_short_token(haystack, needle).or_else(|| haystack.find(needle));
    }

    let mut token_start = None;
    let mut tokens = Vec::new();
    for (index, ch) in haystack.char_indices() {
        if is_token_char(ch) {
            token_start.get_or_insert(index);
        } else if let Some(start) = token_start.take() {
            tokens.push((start, &haystack[start..index]));
        }
    }
    if let Some(start) = token_start {
        tokens.push((start, &haystack[start..]));
    }

    tokens
        .windows(parts.len())
        .find(|window| {
            window
                .iter()
                .zip(&parts)
                .all(|((_, token), part)| token == part)
        })
        .map(|window| window[0].0)
}

fn find_short_token(haystack: &str, needle: &str) -> Option<usize> {
    let mut token_start = None;
    for (index, ch) in haystack.char_indices() {
        if is_token_char(ch) {
            token_start.get_or_insert(index);
            continue;
        }
        if let Some(start) = token_start.take() {
            if &haystack[start..index] == needle {
                return Some(start);
            }
        }
    }
    if let Some(start) = token_start {
        if &haystack[start..] == needle {
            return Some(start);
        }
    }
    None
}

fn is_token_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn text_at(value: &Value, path: &[&str]) -> String {
    let mut current = value;
    for key in path {
        current = &current[*key];
    }
    current.as_str().unwrap_or_default().trim().to_string()
}

fn content_text(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return text.to_string();
    }
    let Some(items) = value.as_array() else {
        return String::new();
    };
    items
        .iter()
        .filter_map(|item| item["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn claude_message_text(value: &Value) -> String {
    let direct = content_text(&value["content"]);
    if !direct.is_empty() {
        return direct;
    }
    let nested = content_text(&value["message"]["content"]);
    if !nested.is_empty() {
        return nested;
    }
    text_at(value, &["message", "content"])
}

fn claude_row_cwd(value: &Value) -> String {
    let cwd = text_at(value, &["cwd"]);
    if !cwd.is_empty() {
        return cwd;
    }
    text_at(value, &["project"])
}

fn compact_function_call(payload: &Value) -> String {
    let name = text_at(payload, &["name"]);
    let args = text_at(payload, &["arguments"]);
    if let Ok(json) = serde_json::from_str::<Value>(&args) {
        let mut parts = vec![name];
        for key in [
            "cmd",
            "command",
            "description",
            "message",
            "objective",
            "prompt",
            "query",
            "task",
            "workdir",
            "path",
        ] {
            let value = text_at(&json, &[key]);
            if !value.is_empty() {
                parts.push(format!("{key}: {}", trim(&value, 1_200)));
            }
        }
        return parts.join("\n");
    }
    format!("{name}\n{args}")
}

fn compact_subagent_notification(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let json_text = trimmed
        .strip_prefix("<subagent_notification>")?
        .strip_suffix("</subagent_notification>")
        .unwrap_or(trimmed)
        .trim();
    let Ok(value) = serde_json::from_str::<Value>(json_text) else {
        return Some(format!(
            "worker notification\n{}",
            trim(json_text.trim_matches(|ch| ch == '<' || ch == '>'), 2_000)
        ));
    };
    let mut parts = vec!["worker notification".to_string()];
    for (label, path) in [
        ("agent", &["agent_path"][..]),
        ("id", &["agent_id"][..]),
        ("thread", &["thread_id"][..]),
        ("session", &["session_id"][..]),
        ("completed", &["status", "completed"][..]),
        ("failed", &["status", "failed"][..]),
        ("error", &["status", "error"][..]),
        ("message", &["status", "message"][..]),
        ("current", &["status", "current_message"][..]),
    ] {
        let value = text_at(&value, path);
        if !value.is_empty() {
            parts.push(format!("{label}: {}", trim(&value, 2_000)));
        }
    }
    Some(parts.join("\n"))
}

fn compact_custom_tool_call(payload: &Value) -> String {
    let name = text_at(payload, &["name"]);
    let mut parts = vec![name];
    let input = &payload["input"];
    if let Some(text) = input.as_str() {
        if !text.trim().is_empty() {
            parts.extend(extract_patch_file_headers(text));
            parts.push(format!("input: {}", trim(text, 1_200)));
        }
    } else if input.is_object() {
        for key in [
            "cmd",
            "command",
            "description",
            "file",
            "filePath",
            "filename",
            "path",
            "query",
            "workdir",
        ] {
            let value = text_at(input, &[key]);
            if !value.is_empty() {
                parts.push(format!("{key}: {value}"));
            }
        }
    }
    parts.join("\n")
}

fn extract_patch_file_headers(text: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        for prefix in ["*** Add File: ", "*** Delete File: ", "*** Update File: "] {
            if let Some(path) = trimmed.strip_prefix(prefix) {
                if !path.trim().is_empty() {
                    paths.push(format!("patch_file: {}", path.trim()));
                }
            }
        }
    }
    paths
}

fn compact_claude_tool(value: &Value) -> String {
    let name = text_at(value, &["tool_name"]);
    let input = &value["tool_input"];
    let mut parts = vec![name];
    for key in [
        "command",
        "description",
        "filePath",
        "file_path",
        "path",
        "workdir",
        "pattern",
    ] {
        let value = text_at(input, &[key]);
        if !value.is_empty() {
            parts.push(format!("{key}: {value}"));
        }
    }
    parts.join("\n")
}

fn claude_nested_tool_docs(value: &Value) -> Vec<(&'static str, &'static str, String)> {
    let Some(items) = value["message"]["content"].as_array() else {
        return Vec::new();
    };
    let mut docs = Vec::new();
    for item in items {
        match text_at(item, &["type"]).as_str() {
            "tool_use" => docs.push(("tool", "tool_command", compact_claude_content_tool(item))),
            "tool_result" => {
                let output = content_text(&item["content"]);
                if looks_like_error(&output) {
                    docs.push(("tool", "error_text", trim(&output, ERROR_CHARS)));
                }
            }
            _ => {}
        }
    }
    docs
}

fn compact_claude_content_tool(value: &Value) -> String {
    let name = text_at(value, &["name"]);
    let input = &value["input"];
    let mut parts = vec![name];
    for key in [
        "command",
        "description",
        "filePath",
        "file_path",
        "path",
        "workdir",
        "pattern",
    ] {
        let value = text_at(input, &[key]);
        if !value.is_empty() {
            parts.push(format!("{key}: {value}"));
        }
    }
    parts.join("\n")
}

fn claude_tool_output(value: &Value) -> String {
    let output = &value["tool_output"];
    for key in ["output", "preview", "error", "stderr"] {
        let text = text_at(output, &[key]);
        if !text.is_empty() {
            return text;
        }
    }
    String::new()
}

fn custom_tool_output_text(payload: &Value) -> String {
    let output = &payload["output"];
    if let Some(text) = output.as_str() {
        if let Ok(json) = serde_json::from_str::<Value>(text) {
            for path in [
                &["output"][..],
                &["error"][..],
                &["stderr"][..],
                &["metadata", "stderr"][..],
            ] {
                let nested = text_at(&json, path);
                if !nested.is_empty() {
                    return nested;
                }
            }
        }
        return text.to_string();
    }
    for key in ["output", "preview", "error", "stderr"] {
        let text = text_at(output, &[key]);
        if !text.is_empty() {
            return text;
        }
    }
    String::new()
}

fn event_msg_tool_output(payload: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(code) = event_msg_exit_code(payload) {
        if code != 0 {
            parts.push(format!("Exit code: {code}"));
        }
    }
    for key in ["aggregated_output", "formatted_output", "stderr", "stdout"] {
        let text = text_at(payload, &[key]);
        if !text.is_empty() {
            parts.push(text);
        }
    }
    parts.join("\n")
}

fn event_msg_exit_code(payload: &Value) -> Option<i64> {
    payload["exit_code"]
        .as_i64()
        .or_else(|| text_at(payload, &["exit_code"]).parse().ok())
}

fn looks_like_error(text: &str) -> bool {
    let lower = text.to_lowercase();
    for marker in [
        "process exited with code:",
        "process exited with code",
        "exit code:",
        "exit code",
    ] {
        if let Some(code) = exit_code_after(&lower, marker) {
            return code != 0;
        }
    }
    [
        "exception",
        "error:",
        "error[",
        "failed:",
        "traceback",
        "panic",
        "panicked",
        "denied",
        "permission denied",
        "no such file",
        "test result: failed",
        "compilation failed",
        "command failed",
        "fatal:",
    ]
    .iter()
    .any(|word| lower.contains(word))
}

fn exit_code_after(text: &str, marker: &str) -> Option<i64> {
    let after = text
        .find(marker)
        .map(|index| &text[index + marker.len()..])?;
    let code = after
        .trim_start_matches(|ch: char| ch.is_whitespace() || ch == ':')
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_matches(|ch: char| !ch.is_ascii_digit() && ch != '-');
    if code.is_empty() {
        None
    } else {
        code.parse().ok()
    }
}

fn should_skip_user_text(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with("# AGENTS.md instructions")
        || trimmed.starts_with("<environment_context>")
        || trimmed.starts_with("<permissions instructions>")
        || trimmed.starts_with("<skill>")
        || trimmed.starts_with("<subagent_notification>")
}

fn normalize_user_text(text: &str) -> String {
    let mut remaining = text.trim_start();
    loop {
        if remaining.starts_with("# AGENTS.md instructions") {
            if let Some(rest) = strip_through(remaining, "</INSTRUCTIONS>") {
                remaining = rest.trim_start();
                continue;
            }
            let Some(index) = remaining.find("\n\n") else {
                return String::new();
            };
            remaining = remaining[index..].trim_start();
            continue;
        }
        if remaining.starts_with("<INSTRUCTIONS>") {
            if let Some(rest) = strip_through(remaining, "</INSTRUCTIONS>") {
                remaining = rest.trim_start();
                continue;
            }
        }
        if remaining.starts_with("<environment_context>") {
            if let Some(rest) = strip_through(remaining, "</environment_context>") {
                remaining = rest.trim_start();
                continue;
            }
        }
        if remaining.starts_with("<permissions instructions>") {
            if let Some(rest) = strip_through(remaining, "</permissions instructions>") {
                remaining = rest.trim_start();
                continue;
            }
        }
        return remaining.to_string();
    }
}

fn strip_through<'a>(text: &'a str, end_marker: &str) -> Option<&'a str> {
    let index = text.find(end_marker)?;
    Some(&text[index + end_marker.len()..])
}

fn session_id_from_path(path: &Path) -> String {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    for part in stem.split('-') {
        if part.len() == 36 {
            return part.to_string();
        }
    }
    stem.to_string()
}

fn repo_name(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

fn first_line(text: &str) -> String {
    if let Some(start) = text.find("<input>") {
        let after_start = &text[start + "<input>".len()..];
        if let Some(end) = after_start.find("</input>") {
            let inner = after_start[..end].trim();
            if !inner.is_empty() {
                return trim(inner.lines().next().unwrap_or_default(), 120);
            }
        }
    }
    trim(text.lines().next().unwrap_or_default(), 120)
}

fn trim(text: &str, limit: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.len() <= limit {
        compact
    } else {
        let safe_limit = limit.saturating_sub(3);
        let end = compact
            .char_indices()
            .map(|(index, _)| index)
            .take_while(|index| *index <= safe_limit)
            .last()
            .unwrap_or(0);
        format!("{}...", &compact[..end])
    }
}

fn one_line(text: &str, limit: usize) -> String {
    let text = if text.is_empty() { "unknown" } else { text };
    trim(text, limit)
}

fn safe_slice(text: &str, start: usize, end: usize) -> &str {
    let start = previous_char_boundary(text, start.min(text.len()));
    let end = previous_char_boundary(text, end.min(text.len()));
    &text[start..end]
}

fn previous_char_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    if text.is_char_boundary(index) {
        return index;
    }
    text.char_indices()
        .map(|(boundary, _)| boundary)
        .take_while(|boundary| *boundary < index)
        .last()
        .unwrap_or(0)
}

fn previous_word_boundary(text: &str, index: usize) -> usize {
    let index = previous_char_boundary(text, index.min(text.len()));
    let mut boundary = 0;
    for (cursor, ch) in text.char_indices() {
        if cursor >= index {
            break;
        }
        if ch.is_whitespace() {
            boundary = cursor + ch.len_utf8();
        }
    }
    boundary
}

fn file_mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

fn source_is_recent(path: &Path, duration: Duration) -> bool {
    let Some(cutoff) = SystemTime::now().checked_sub(duration) else {
        return true;
    };
    file_mtime(path).unwrap_or(UNIX_EPOCH) >= cutoff
}

fn source_recency_key(path: &Path) -> i64 {
    file_mtime(path)
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

fn mtime_ms(metadata: &fs::Metadata) -> AppResult<i64> {
    Ok(metadata.modified()?.duration_since(UNIX_EPOCH)?.as_millis() as i64)
}

fn mtime_ns(metadata: &fs::Metadata) -> AppResult<i64> {
    Ok(metadata
        .modified()?
        .duration_since(UNIX_EPOCH)?
        .as_nanos()
        .min(i64::MAX as u128) as i64)
}

#[cfg(test)]
mod tests {
    use super::{looks_like_error, matched_terms, query_terms, trim, SearchRow};

    #[test]
    fn trim_does_not_split_unicode_chars() {
        let trimmed = trim("abc ───── def", 8);
        assert!(trimmed.ends_with("..."));
    }

    #[test]
    fn short_terms_match_token_boundaries() {
        let row = SearchRow {
            session_id: "s".to_string(),
            title: "systemic auditability visible".to_string(),
            cwd: "/tmp/example_repo".to_string(),
            updated_at: String::new(),
            source_path: "/tmp/rollout.jsonl".to_string(),
            session_kind: "full".to_string(),
            parent_thread_id: String::new(),
            category: "conversation_window".to_string(),
            body: "This talks about export types and auditability, not the UI.".to_string(),
            rank: 0.0,
        };
        let matched = matched_terms(
            &row,
            &[
                "example_repo".to_string(),
                "export".to_string(),
                "ui".to_string(),
            ],
        );
        assert_eq!(matched, vec!["example_repo", "export", "ui"]);

        let noisy = SearchRow {
            body: "auditability guidelines exportable".to_string(),
            ..row
        };
        let matched = matched_terms(&noisy, &["ui".to_string()]);
        assert!(matched.is_empty(), "{matched:?}");
    }

    #[test]
    fn hyphenated_terms_match_token_sequences() {
        let row = SearchRow {
            session_id: "s".to_string(),
            title: "Review helper".to_string(),
            cwd: "/tmp/project".to_string(),
            updated_at: String::new(),
            source_path: "/tmp/rollout.jsonl".to_string(),
            session_kind: "full".to_string(),
            parent_thread_id: String::new(),
            category: "conversation_window".to_string(),
            body: "Please run the test audit pass before shipping.".to_string(),
            rank: 0.0,
        };
        let matched = matched_terms(&row, &["test-audit".to_string()]);
        assert_eq!(matched, vec!["test-audit"]);

        let noisy = SearchRow {
            body: "Please test the installer and audit the results.".to_string(),
            ..row
        };
        let matched = matched_terms(&noisy, &["test-audit".to_string()]);
        assert!(matched.is_empty(), "{matched:?}");
    }

    #[test]
    fn successful_tool_output_with_code_symbols_is_not_error_text() {
        assert!(!looks_like_error(
            "Process exited with code 0\nOutput:\nexport const ApiError = z.string();"
        ));
        assert!(!looks_like_error(
            "Exit code: 0\nOutput:\ncommand failed is just source text"
        ));
        assert!(looks_like_error(
            "Process exited with code 101\nOutput:\nerror[E0425]: cannot find value"
        ));
        assert!(looks_like_error("Exit code: 1\nOutput:\nplain failure"));
        assert!(looks_like_error(
            "process exited with code: 2\nOutput:\nplain failure"
        ));
    }

    #[test]
    fn query_terms_drop_operational_stopwords() {
        assert_eq!(
            query_terms("You are doing the mandatory test-audit subagent"),
            vec!["mandatory", "test-audit", "subagent"]
        );
    }
}
