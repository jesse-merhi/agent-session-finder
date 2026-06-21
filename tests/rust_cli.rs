use rusqlite::Connection;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[test]
fn install_script_copies_rust_binary_to_requested_bin_dir() {
    let root = unique_temp_dir();
    let bin_dir = root.join("bin");
    let install_script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("install.sh");

    let status = Command::new("sh")
        .arg(install_script)
        .args([
            "--bin-dir",
            bin_dir.to_str().unwrap(),
            "--profile",
            "debug",
            "--no-build",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let installed = bin_dir.join("agent-session-find");
    let validator = bin_dir.join("agent-skill-validate");
    assert!(installed.is_file());
    assert!(validator.is_file());

    let output = Command::new(installed).arg("--help").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("Local lightweight session finder"),
        "{stdout}"
    );

    let output = Command::new(validator).arg("--help").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("Validate local Codex skill folders"),
        "{stdout}"
    );
}

#[test]
fn install_script_honors_cargo_target_dir() {
    let root = unique_temp_dir();
    let bin_dir = root.join("bin");
    let target_dir = root.join("custom-target");
    let debug_dir = target_dir.join("debug");
    fs::create_dir_all(&debug_dir).unwrap();
    fs::copy(
        env!("CARGO_BIN_EXE_agent-session-find"),
        debug_dir.join("agent-session-find"),
    )
    .unwrap();
    fs::copy(
        env!("CARGO_BIN_EXE_agent-skill-validate"),
        debug_dir.join("agent-skill-validate"),
    )
    .unwrap();

    let install_script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("install.sh");
    let status = Command::new("sh")
        .arg(install_script)
        .env("CARGO_TARGET_DIR", target_dir.to_str().unwrap())
        .args([
            "--bin-dir",
            bin_dir.to_str().unwrap(),
            "--profile",
            "debug",
            "--no-build",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let installed = bin_dir.join("agent-session-find");
    let validator = bin_dir.join("agent-skill-validate");
    assert!(installed.is_file());
    assert!(validator.is_file());

    let output = Command::new(installed).arg("--help").output().unwrap();
    assert!(output.status.success());

    let output = Command::new(validator).arg("--help").output().unwrap();
    assert!(output.status.success());
}

#[test]
fn ranks_low_token_intent_results_for_example_repo_export_ui() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");

    let wanted =
        sessions.join("rollout-2026-06-20T01-00-00-11111111-1111-4111-8111-111111111111.jsonl");
    fs::write(
        &wanted,
        lines(&[
            r#"{"timestamp":"2026-06-20T01:00:00Z","type":"session_meta","payload":{"id":"11111111-1111-4111-8111-111111111111","cwd":"/Users/jessemerhi/repos/example_repo"}}"#,
            r#"{"timestamp":"2026-06-20T01:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"Find the example_repo export UI session and fix the broken filters."}}"#,
            r#"{"timestamp":"2026-06-20T01:00:02Z","type":"event_msg","payload":{"type":"agent_message","message":"I found the export UI path and the filter bug."}}"#,
        ]),
    )
    .unwrap();

    let noisy =
        sessions.join("rollout-2026-06-20T01-05-00-22222222-2222-4222-8222-222222222222.jsonl");
    fs::write(
        &noisy,
        lines(&[
            r#"{"timestamp":"2026-06-20T01:05:00Z","type":"session_meta","payload":{"id":"22222222-2222-4222-8222-222222222222","cwd":"/Users/jessemerhi/repos/example_repo"}}"#,
            r#"{"timestamp":"2026-06-20T01:05:01Z","type":"event_msg","payload":{"type":"user_message","message":"Review generated files."}}"#,
            r#"{"timestamp":"2026-06-20T01:05:02Z","type":"response_item","payload":{"type":"function_call_output","output":"export const ui = 1;\nexport type ExportThing = string;\nexport const MoreUi = true;"}}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "example_repo export ui",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();
    let first = stdout.lines().next().unwrap_or_default();
    assert!(first.contains("example_repo export UI"), "{stdout}");
    assert!(stdout.contains("match: 3/3"), "{stdout}");
    assert!(stdout.lines().count() < 40, "{stdout}");
}

#[test]
fn search_output_handles_broken_pipe() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        sessions.join("rollout-2026-06-20T01-10-00-33333333-3333-4333-8333-333333333333.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T01:10:00Z","type":"session_meta","payload":{"id":"33333333-3333-4333-8333-333333333333","cwd":"/Users/jessemerhi/repos/example_repo"}}"#,
            r#"{"timestamp":"2026-06-20T01:10:01Z","type":"event_msg","payload":{"type":"user_message","message":"Find the example_repo export UI session and explain the search result format."}}"#,
            r#"{"timestamp":"2026-06-20T01:10:02Z","type":"event_msg","payload":{"type":"agent_message","message":"The search output should stay pipeline friendly."}}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let mut child = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "example_repo export ui",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take().unwrap());

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn deduplicates_codex_response_item_and_event_user_prompts() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        sessions.join("rollout-2026-06-20T01-30-00-12121212-1212-4212-8212-121212121212.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T01:30:00Z","type":"session_meta","payload":{"id":"12121212-1212-4212-8212-121212121212","cwd":"/repo"}}"#,
            r#"{"timestamp":"2026-06-20T01:30:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"find duplicate banana"}]}}"#,
            r#"{"timestamp":"2026-06-20T01:30:01Z","type":"event_msg","payload":{"type":"user_message","message":"find duplicate banana"}}"#,
            r#"{"timestamp":"2026-06-20T01:30:02Z","type":"event_msg","payload":{"type":"agent_message","message":"assistant context unique"}}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let conn = Connection::open(&db).unwrap();
    let windows: Vec<String> = conn
        .prepare("select body from docs where category='conversation_window'")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(windows.len(), 1, "{windows:?}");
    assert!(windows[0].contains("find duplicate banana"), "{windows:?}");
    assert!(
        windows[0].contains("assistant context unique"),
        "{windows:?}"
    );
}

#[test]
fn deduplicates_adjacent_codex_user_events_with_different_timestamps() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        sessions.join("rollout-2026-06-20T01-35-00-12121212-1212-4212-8212-121212121213.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T01:35:00Z","type":"session_meta","payload":{"id":"12121212-1212-4212-8212-121212121213","cwd":"/repo"}}"#,
            r#"{"timestamp":"2026-06-20T01:35:01.001Z","type":"event_msg","payload":{"type":"user_message","message":"find duplicate grape"}}"#,
            r#"{"timestamp":"2026-06-20T01:35:01.004Z","type":"event_msg","payload":{"type":"user_message","message":"find duplicate grape"}}"#,
            r#"{"timestamp":"2026-06-20T01:35:02Z","type":"event_msg","payload":{"type":"agent_message","message":"assistant context retained"}}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let conn = Connection::open(&db).unwrap();
    let windows: Vec<String> = conn
        .prepare("select body from docs where category='conversation_window'")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(windows.len(), 1, "{windows:?}");
    assert!(windows[0].contains("find duplicate grape"), "{windows:?}");
    assert!(
        windows[0].contains("assistant context retained"),
        "{windows:?}"
    );
}

#[test]
fn indexes_codex_function_call_command_arguments() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        sessions.join("rollout-2026-06-20T01-40-00-15151515-1515-4515-8515-151515151515.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T01:40:00Z","type":"session_meta","payload":{"id":"15151515-1515-4515-8515-151515151515","cwd":"/repo"}}"#,
            r#"{"timestamp":"2026-06-20T01:40:01Z","type":"event_msg","payload":{"type":"user_message","message":"Read the command-only path."}}"#,
            r#"{"timestamp":"2026-06-20T01:40:02Z","type":"response_item","payload":{"type":"function_call","name":"shell_command","arguments":"{\"command\":\"cat src/command_only_path.rs\",\"workdir\":\"/repo\"}"}}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "command_only_path",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("command_only_path"), "{stdout}");
    assert!(
        stdout.contains("15151515-1515-4515-8515-151515151515"),
        "{stdout}"
    );
}

#[test]
fn indexes_codex_custom_tool_call_paths() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        sessions.join("rollout-2026-06-20T01-45-00-13131313-1313-4313-8313-131313131313.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T01:45:00Z","type":"session_meta","payload":{"id":"13131313-1313-4313-8313-131313131313","cwd":"/repo"}}"#,
            r#"{"timestamp":"2026-06-20T01:45:01Z","type":"event_msg","payload":{"type":"user_message","message":"Patch the special file."}}"#,
            r#"{"timestamp":"2026-06-20T01:45:02Z","type":"response_item","payload":{"type":"custom_tool_call","name":"apply_patch","input":"*** Begin Patch\n*** Update File: src/special_custom_path.rs\n"}}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "special_custom_path",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("special_custom_path"), "{stdout}");
    assert!(
        stdout.contains("13131313-1313-4313-8313-131313131313"),
        "{stdout}"
    );
}

#[test]
fn indexes_codex_patch_headers_after_long_patch_bodies() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");
    let long_patch_body = "x".repeat(1_300);
    let patch_input = format!(
        "*** Begin Patch\n*** Update File: src/first_file.rs\n+{}\n*** Update File: src/second_special_file.rs\n+ok\n*** End Patch",
        long_patch_body
    );
    let tool_call = format!(
        r#"{{"timestamp":"2026-06-20T01:46:02Z","type":"response_item","payload":{{"type":"custom_tool_call","name":"apply_patch","input":{}}}}}"#,
        serde_json::to_string(&patch_input).unwrap()
    );

    fs::write(
        sessions.join("rollout-2026-06-20T01-46-00-14141414-1414-4414-8414-141414141414.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T01:46:00Z","type":"session_meta","payload":{"id":"14141414-1414-4414-8414-141414141414","cwd":"/repo"}}"#,
            r#"{"timestamp":"2026-06-20T01:46:01Z","type":"event_msg","payload":{"type":"user_message","message":"Patch several files."}}"#,
            &tool_call,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "second_special_file",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("second_special_file"), "{stdout}");
    assert!(
        stdout.contains("14141414-1414-4414-8414-141414141414"),
        "{stdout}"
    );
}

#[test]
fn path_flags_expand_home_prefixes() {
    let root = unique_temp_dir();
    let home = root.join("home");
    let sessions = home.join("codex/sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();

    fs::write(
        sessions.join("rollout-2026-06-20T01-55-00-14141414-1414-4414-8414-141414141414.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T01:55:00Z","type":"session_meta","payload":{"id":"14141414-1414-4414-8414-141414141414","cwd":"/repo"}}"#,
            r#"{"timestamp":"2026-06-20T01:55:01Z","type":"event_msg","payload":{"type":"user_message","message":"tilde expanded path needle"}}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .env("HOME", &home)
        .args([
            "--codex-home",
            "~/codex",
            "--db",
            "~/idx.sqlite",
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());
    assert!(home.join("idx.sqlite").is_file());
    assert!(!root.join("~").exists());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .env("HOME", &home)
        .args([
            "--codex-home",
            "~/codex",
            "--db",
            "~/idx.sqlite",
            "--source",
            "codex",
            "--no-refresh",
            "tilde expanded path needle",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("14141414-1414-4414-8414-141414141414"),
        "{stdout}"
    );
}

#[test]
fn prefers_full_sessions_over_matching_subagents() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");

    let full =
        sessions.join("rollout-2026-06-20T02-00-00-33333333-3333-4333-8333-333333333333.jsonl");
    fs::write(
        &full,
        lines(&[
            r#"{"timestamp":"2026-06-20T02:00:00Z","type":"session_meta","payload":{"id":"33333333-3333-4333-8333-333333333333","cwd":"/Users/jessemerhi/repos/example_repo","thread_source":"user"}}"#,
            r#"{"timestamp":"2026-06-20T02:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"Ok let's fix the example_repo export UI bugs in the main session."}}"#,
            r#"{"timestamp":"2026-06-20T02:00:02Z","type":"event_msg","payload":{"type":"agent_message","message":"I will inspect the export UI and keep this session as the main handoff point."}}"#,
        ]),
    )
    .unwrap();

    let subagent =
        sessions.join("rollout-2026-06-20T02-05-00-44444444-4444-4444-8444-444444444444.jsonl");
    fs::write(
        &subagent,
        lines(&[
            r#"{"timestamp":"2026-06-20T02:05:00Z","type":"session_meta","payload":{"id":"44444444-4444-4444-8444-444444444444","parent_thread_id":"33333333-3333-4333-8333-333333333333","cwd":"/Users/jessemerhi/.codex/worktrees/abcd/example_repo","thread_source":"subagent","source":{"subagent":{"thread_spawn":{"parent_thread_id":"33333333-3333-4333-8333-333333333333","depth":1}}}}}"#,
            r#"{"timestamp":"2026-06-20T02:05:01Z","type":"event_msg","payload":{"type":"user_message","message":"You are doing the mandatory test-audit subagent pass for Example Repo export UI bugs. Report only findings."}}"#,
            r#"{"timestamp":"2026-06-20T02:05:02Z","type":"event_msg","payload":{"type":"agent_message","message":"Export UI bug bug bug export UI Example Repo."}}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "--limit",
            "5",
            "example_repo export ui bugs",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();
    let first = stdout.lines().next().unwrap_or_default();
    assert!(first.contains("main session"), "{stdout}");
    assert!(stdout.contains("session: full"), "{stdout}");
    assert!(!stdout.contains("session: subagent"), "{stdout}");
}

#[test]
fn does_not_index_subagent_content() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");

    let full =
        sessions.join("rollout-2026-06-20T03-00-00-55555555-5555-4555-8555-555555555555.jsonl");
    fs::write(
        &full,
        lines(&[
            r#"{"timestamp":"2026-06-20T03:00:00Z","type":"session_meta","payload":{"id":"55555555-5555-4555-8555-555555555555","cwd":"/Users/jessemerhi/repos/example_repo","thread_source":"user"}}"#,
            r#"{"timestamp":"2026-06-20T03:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"This is the main Example Repo session."}}"#,
        ]),
    )
    .unwrap();

    let subagent =
        sessions.join("rollout-2026-06-20T03-05-00-66666666-6666-4666-8666-666666666666.jsonl");
    fs::write(
        &subagent,
        lines(&[
            r#"{"timestamp":"2026-06-20T03:05:00Z","type":"session_meta","payload":{"id":"66666666-6666-4666-8666-666666666666","parent_thread_id":"55555555-5555-4555-8555-555555555555","cwd":"/Users/jessemerhi/.codex/worktrees/abcd/example_repo","thread_source":"subagent","source":{"subagent":{"thread_spawn":{"parent_thread_id":"55555555-5555-4555-8555-555555555555","depth":1}}}}}"#,
            r#"{"timestamp":"2026-06-20T03:05:01Z","type":"event_msg","payload":{"type":"user_message","message":"Export UI bugs only appear in this subagent worker."}}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "export ui bugs",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("No matching sessions found"), "{stdout}");
}

#[test]
fn does_not_index_thread_source_only_subagents() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        sessions.join("rollout-2026-06-20T03-30-00-aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T03:30:00Z","type":"session_meta","payload":{"id":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","cwd":"/repo/example_repo","thread_source":"subagent"}}"#,
            r#"{"timestamp":"2026-06-20T03:30:01Z","type":"event_msg","payload":{"type":"user_message","message":"thread source only worker leakage needle"}}"#,
        ]),
    )
    .unwrap();

    let index_output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .output()
        .unwrap();
    assert!(index_output.status.success());
    let stdout = String::from_utf8(index_output.stdout).unwrap();
    assert!(stdout.contains("skipped 0 unchanged sources"), "{stdout}");
    assert!(stdout.contains("filtered 1 sources"), "{stdout}");

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "thread source only worker leakage needle",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("No matching sessions found"), "{stdout}");
}

#[test]
fn does_not_index_string_source_subagents() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        sessions.join("rollout-2026-06-20T03-35-00-aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaab.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T03:35:00Z","type":"session_meta","payload":{"id":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaab","cwd":"/repo/example_repo","source":{"subagent":"review"}}}"#,
            r#"{"timestamp":"2026-06-20T03:35:01Z","type":"event_msg","payload":{"type":"user_message","message":"string subagent leakage needle"}}"#,
        ]),
    )
    .unwrap();

    let index_output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .output()
        .unwrap();
    assert!(index_output.status.success());
    let stdout = String::from_utf8(index_output.stdout).unwrap();
    assert!(stdout.contains("skipped 0 unchanged sources"), "{stdout}");
    assert!(stdout.contains("filtered 1 sources"), "{stdout}");

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "string subagent leakage needle",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("No matching sessions found"), "{stdout}");
}

#[test]
fn indexes_text_only_review_prompts_as_full_sessions() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");

    let worker =
        sessions.join("rollout-2026-06-20T04-00-00-77777777-7777-4777-8777-777777777777.jsonl");
    fs::write(
        &worker,
        lines(&[
            r#"{"timestamp":"2026-06-20T04:00:00Z","type":"session_meta","payload":{"id":"77777777-7777-4777-8777-777777777777","cwd":"/Users/jessemerhi/repos/example_repo","thread_source":"user"}}"#,
            r#"{"timestamp":"2026-06-20T04:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"You are reviewing PR #123 and doing the mandatory test-audit pass. Report export UI bugs only."}}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "export UI bugs",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("session: full"), "{stdout}");
    assert!(stdout.contains("You are reviewing PR #123"), "{stdout}");
    assert!(
        stdout.contains("77777777-7777-4777-8777-777777777777"),
        "{stdout}"
    );
}

#[test]
fn indexes_top_level_review_sessions() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        sessions.join("rollout-2026-06-20T04-30-00-77777777-7777-4777-8777-777777777778.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T04:30:00Z","type":"session_meta","payload":{"id":"77777777-7777-4777-8777-777777777778","cwd":"/repo/example_repo","thread_source":"user"}}"#,
            r#"{"timestamp":"2026-06-20T04:30:01Z","type":"event_msg","payload":{"type":"user_message","message":"Review the current code changes. Do not edit files. Report: prioritized findings about banana export bugs."}}"#,
            r#"{"timestamp":"2026-06-20T04:30:02Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<user_action><context>User initiated a review task. Here's the full review output from reviewer model.</context></user_action>"}]}}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "current code changes",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("session: full"), "{stdout}");
    assert!(stdout.contains("banana export bugs"), "{stdout}");
    assert!(
        stdout.contains("77777777-7777-4777-8777-777777777778"),
        "{stdout}"
    );
}

#[test]
fn indexes_user_prompt_after_harness_preamble() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        sessions.join("rollout-2026-06-20T04-40-00-77777777-7777-4777-8777-777777777779.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T04:40:00Z","type":"session_meta","payload":{"id":"77777777-7777-4777-8777-777777777779","cwd":"/repo/example_repo","thread_source":"user"}}"#,
            "{\"timestamp\":\"2026-06-20T04:40:01Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"# AGENTS.md instructions\\n\\n<INSTRUCTIONS>\\n# Global Agent Rules\\n\\nThese rules apply.\\n</INSTRUCTIONS><environment_context>\\nrepo context\\n</environment_context>Review the current code changes and find the banana export bug.\"}}",
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "current code changes",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let first = stdout.lines().next().unwrap_or_default();
    assert!(
        first.contains("Review the current code changes"),
        "{stdout}"
    );
    assert!(!stdout.contains("<INSTRUCTIONS>"), "{stdout}");
    assert!(stdout.contains("session: full"), "{stdout}");
    assert!(stdout.contains("banana export bug"), "{stdout}");
}

#[test]
fn indexes_top_level_read_only_cold_review_sessions() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        sessions.join("rollout-2026-06-20T04-40-00-77777777-7777-4777-8777-777777777779.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T04:40:00Z","type":"session_meta","payload":{"id":"77777777-7777-4777-8777-777777777779","cwd":"/repo/example_repo","thread_source":"user"}}"#,
            r#"{"timestamp":"2026-06-20T04:40:01Z","type":"event_msg","payload":{"type":"user_message","message":"Please run a read-only cold review of this change for frobnicate bugs."}}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "frobnicate",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("session: full"), "{stdout}");
    assert!(stdout.contains("read-only cold review"), "{stdout}");
}

#[test]
fn hyphenated_queries_match_tokenized_session_text() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        sessions.join("rollout-2026-06-20T04-45-00-77777777-7777-4777-8777-777777777781.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T04:45:00Z","type":"session_meta","payload":{"id":"77777777-7777-4777-8777-777777777781","cwd":"/repo/example_repo","thread_source":"user"}}"#,
            r#"{"timestamp":"2026-06-20T04:45:01Z","type":"event_msg","payload":{"type":"user_message","message":"Please run the test audit pass for banana coverage."}}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "test-audit",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("match: 1/1"), "{stdout}");
    assert!(stdout.contains("terms: test-audit"), "{stdout}");
    assert!(stdout.contains("test audit pass"), "{stdout}");
}

#[test]
fn indexes_top_level_skill_requests() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        sessions.join("rollout-2026-06-20T04-50-00-77777777-7777-4777-8777-777777777780.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T04:50:00Z","type":"session_meta","payload":{"id":"77777777-7777-4777-8777-777777777780","cwd":"/repo/example_repo","thread_source":"user"}}"#,
            r#"{"timestamp":"2026-06-20T04:50:01Z","type":"event_msg","payload":{"type":"user_message","message":"Use the test-audit skill to inspect banana coverage in this top-level session."}}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "banana coverage",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("session: full"), "{stdout}");
    assert!(stdout.contains("banana coverage"), "{stdout}");
}

#[test]
fn skips_internal_skill_load_payloads() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        sessions.join("rollout-2026-06-20T04-55-00-77777777-7777-4777-8777-777777777782.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T04:55:00Z","type":"session_meta","payload":{"id":"77777777-7777-4777-8777-777777777782","cwd":"/repo/example_repo","thread_source":"user"}}"#,
            r#"{"timestamp":"2026-06-20T04:55:01Z","type":"event_msg","payload":{"type":"user_message","message":"<skill>\n<name>finding-discipline</name>\n<path>/skills/finding-discipline/SKILL.md</path>\n---\nThis loaded skill mentions banana spectral coverage only inside harness context.\n</skill>"}}"#,
            r#"{"timestamp":"2026-06-20T04:55:02Z","type":"event_msg","payload":{"type":"user_message","message":"Real user prompt about unrelated apple work."}}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let skill_hit = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "banana spectral coverage",
        ])
        .output()
        .unwrap();
    assert!(!skill_hit.status.success());
    let stdout = String::from_utf8(skill_hit.stdout).unwrap();
    assert!(stdout.contains("No matching sessions found"), "{stdout}");

    let real_prompt = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "apple work",
        ])
        .output()
        .unwrap();
    assert!(real_prompt.status.success());
    let stdout = String::from_utf8(real_prompt.stdout).unwrap();
    assert!(stdout.contains("Real user prompt"), "{stdout}");
}

#[test]
fn does_not_index_subagent_notification_payloads() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        sessions.join("rollout-2026-06-20T05-00-00-88888888-8888-4888-8888-888888888888.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T05:00:00Z","type":"session_meta","payload":{"id":"88888888-8888-4888-8888-888888888888","cwd":"/Users/jessemerhi/repos/example_repo","thread_source":"user"}}"#,
            r#"{"timestamp":"2026-06-20T05:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"This parent session should stay indexed for normal agent context."}}"#,
            r#"{"timestamp":"2026-06-20T05:00:02Z","type":"event_msg","payload":{"type":"user_message","message":"<subagent_notification>{\"agent_path\":\"worker\",\"status\":{\"completed\":\"notification only spectral banana phrase\"}}</subagent_notification>"}}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let parent = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "normal agent context",
        ])
        .output()
        .unwrap();
    assert!(parent.status.success());

    let notification = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "spectral banana phrase",
        ])
        .output()
        .unwrap();
    assert!(!notification.status.success());
    let stdout = String::from_utf8(notification.stdout).unwrap();
    assert!(stdout.contains("No matching sessions found"), "{stdout}");
}

#[test]
fn does_not_index_claude_history_rows() {
    let root = unique_temp_dir();
    let claude_home = root.join("claude");
    fs::create_dir_all(&claude_home).unwrap();
    let db = root.join("index.sqlite");
    fs::write(
        claude_home.join("history.jsonl"),
        lines(&[r#"{"display":"/login export ui example_repo"}"#]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "status",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Sessions: 0"), "{stdout}");
    assert!(stdout.contains("Docs: 0"), "{stdout}");
}

#[test]
fn claude_cwd_only_search_uses_transcript_project_metadata() {
    let root = unique_temp_dir();
    let claude_home = root.join("claude");
    let transcripts = claude_home.join("transcripts");
    fs::create_dir_all(&transcripts).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        transcripts.join("ses_project.jsonl"),
        lines(&[r#"{"type":"user","timestamp":"2026-06-20T05:30:00Z","content":"hello from claude","project":"/repo/example_repo"}"#]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "--no-refresh",
            "--cwd",
            "example_repo",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("claude:ses_project"), "{stdout}");
    assert!(stdout.contains("/repo/example_repo"), "{stdout}");
}

#[test]
fn indexes_top_level_claude_content_arrays() {
    let root = unique_temp_dir();
    let claude_home = root.join("claude");
    let transcripts = claude_home.join("transcripts");
    fs::create_dir_all(&transcripts).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        transcripts.join("ses_array.jsonl"),
        lines(&[r#"{"type":"user","timestamp":"2026-06-20T05:35:00Z","content":[{"type":"text","text":"array content needle"}],"project":"/repo/example_repo"}"#]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "--no-refresh",
            "array content needle",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("claude:ses_array"), "{stdout}");
    assert!(stdout.contains("array content needle"), "{stdout}");
}

#[test]
fn skips_claude_internal_skill_load_payloads() {
    let root = unique_temp_dir();
    let claude_home = root.join("claude");
    let transcripts = claude_home.join("transcripts");
    fs::create_dir_all(&transcripts).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        transcripts.join("ses_skill.jsonl"),
        lines(&[
            r#"{"type":"user","timestamp":"2026-06-20T05:36:00Z","content":"<skill>\n<name>finding-discipline</name>\nThis loaded skill mentions banana spectral coverage only inside harness context.\n</skill>","project":"/repo/example_repo"}"#,
            r#"{"type":"user","timestamp":"2026-06-20T05:36:01Z","content":"Real user prompt about apple work.","project":"/repo/example_repo"}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let skill_hit = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "--no-refresh",
            "banana spectral coverage",
        ])
        .output()
        .unwrap();
    assert!(!skill_hit.status.success());
    let stdout = String::from_utf8(skill_hit.stdout).unwrap();
    assert!(stdout.contains("No matching sessions found"), "{stdout}");

    let real_prompt = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "--no-refresh",
            "apple work",
        ])
        .output()
        .unwrap();
    assert!(real_prompt.status.success());
    let stdout = String::from_utf8(real_prompt.stdout).unwrap();
    assert!(stdout.contains("Real user prompt"), "{stdout}");
}

#[test]
fn indexes_claude_project_sessions_without_nested_subagents() {
    let root = unique_temp_dir();
    let claude_home = root.join("claude");
    let project = claude_home.join("projects/-repo-example_repo");
    let subagents = project.join("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa/subagents");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&subagents).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        project.join("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa.jsonl"),
        lines(&[
            r#"{"type":"last-prompt","sessionId":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"}"#,
            r#"{"type":"user","timestamp":"2026-06-20T05:40:00Z","isSidechain":false,"message":{"role":"user","content":"claude project session export ui needle"},"cwd":"/repo/example_repo","sessionId":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"}"#,
            r#"{"type":"assistant","timestamp":"2026-06-20T05:40:01Z","isSidechain":false,"message":{"role":"assistant","content":[{"type":"text","text":"I found the Claude project session."}]},"cwd":"/repo/example_repo","sessionId":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"}"#,
        ]),
    )
    .unwrap();
    fs::write(
        subagents.join("agent-noise.jsonl"),
        lines(&[r#"{"type":"user","timestamp":"2026-06-20T05:41:00Z","isSidechain":true,"message":{"role":"user","content":"nestedsubagentpollutiontoken"},"cwd":"/repo/example_repo","sessionId":"agent-noise"}"#]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let project_hit = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "--no-refresh",
            "claude project session export ui",
        ])
        .output()
        .unwrap();
    assert!(project_hit.status.success());
    let stdout = String::from_utf8(project_hit.stdout).unwrap();
    assert!(
        stdout.contains("claude:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"),
        "{stdout}"
    );
    assert!(stdout.contains("/repo/example_repo"), "{stdout}");

    let subagent_noise = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "--no-refresh",
            "nestedsubagentpollutiontoken",
        ])
        .output()
        .unwrap();
    assert!(!subagent_noise.status.success());
    let stdout = String::from_utf8(subagent_noise.stdout).unwrap();
    assert!(stdout.contains("No matching sessions found"), "{stdout}");
}

#[test]
fn skips_claude_metadata_only_project_files() {
    let root = unique_temp_dir();
    let claude_home = root.join("claude");
    let project = claude_home.join("projects/-repo-example_repo");
    fs::create_dir_all(&project).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        project.join("metadata-only.jsonl"),
        lines(&[r#"{"type":"bridge-session","sessionId":"metadata-only","bridgeSessionId":"cse_123","lastSequenceNum":0}"#]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "--no-refresh",
            "example_repo",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("No matching sessions found"), "{stdout}");

    let conn = Connection::open(&db).unwrap();
    let sessions: i64 = conn
        .query_row("select count(*) from sessions", [], |row| row.get(0))
        .unwrap();
    let docs: i64 = conn
        .query_row("select count(*) from docs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(sessions, 0);
    assert_eq!(docs, 0);
}

#[test]
fn claude_sidechain_rows_do_not_hide_parent_session() {
    let root = unique_temp_dir();
    let claude_home = root.join("claude");
    let project = claude_home.join("projects/-repo-example_repo");
    fs::create_dir_all(&project).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        project.join("cccccccc-cccc-4ccc-8ccc-cccccccccccc.jsonl"),
        lines(&[
            r#"{"type":"user","timestamp":"2026-06-20T05:42:00Z","isSidechain":false,"message":{"role":"user","content":"top level durable claude needle"},"cwd":"/repo/example_repo","sessionId":"cccccccc-cccc-4ccc-8ccc-cccccccccccc"}"#,
            r#"{"type":"assistant","timestamp":"2026-06-20T05:42:01Z","isSidechain":true,"message":{"role":"assistant","content":[{"type":"text","text":"sidechainpollutiontoken"}]},"cwd":"/repo/example_repo","sessionId":"cccccccc-cccc-4ccc-8ccc-cccccccccccc"}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let parent_hit = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "--no-refresh",
            "top level durable claude needle",
        ])
        .output()
        .unwrap();
    assert!(parent_hit.status.success());
    let stdout = String::from_utf8(parent_hit.stdout).unwrap();
    assert!(
        stdout.contains("claude:cccccccc-cccc-4ccc-8ccc-cccccccccccc"),
        "{stdout}"
    );
    assert!(stdout.contains("session: full"), "{stdout}");

    let sidechain_noise = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "--no-refresh",
            "sidechainpollutiontoken",
        ])
        .output()
        .unwrap();
    assert!(!sidechain_noise.status.success());
    let stdout = String::from_utf8(sidechain_noise.stdout).unwrap();
    assert!(stdout.contains("No matching sessions found"), "{stdout}");
}

#[test]
fn custom_claude_home_project_rows_keep_claude_source_kind() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let claude_home = root.join("claude");
    let project = claude_home.join("projects/-repo-example_repo");
    fs::create_dir_all(codex_home.join("sessions")).unwrap();
    fs::create_dir_all(&project).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        project.join("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb.jsonl"),
        lines(&[r#"{"type":"user","timestamp":"2026-06-20T05:45:00Z","message":{"role":"user","content":"custom claude home project needle"},"cwd":"/repo/example_repo","sessionId":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"}"#]),
    )
    .unwrap();

    for source in ["claude", "codex"] {
        let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
            .args([
                "--codex-home",
                codex_home.to_str().unwrap(),
                "--claude-home",
                claude_home.to_str().unwrap(),
                "--db",
                db.to_str().unwrap(),
                "--source",
                source,
                "index",
            ])
            .status()
            .unwrap();
        assert!(status.success(), "{source}");
    }

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "--no-refresh",
            "custom claude home project needle",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("claude:bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"),
        "{stdout}"
    );
}

#[test]
fn indexes_claude_tool_input_paths() {
    let root = unique_temp_dir();
    let claude_home = root.join("claude");
    let transcripts = claude_home.join("transcripts");
    fs::create_dir_all(&transcripts).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        transcripts.join("ses_tool_path.jsonl"),
        lines(&[
            r#"{"type":"user","timestamp":"2026-06-20T05:47:00Z","content":"look for the export route","project":"/repo/example_repo"}"#,
            r#"{"type":"tool_use","timestamp":"2026-06-20T05:47:01Z","tool_name":"grep","tool_input":{"pattern":"ExportRoute","file_path":"src/export/special_file.rs","workdir":"/repo/example_repo"}}"#,
            r#"{"type":"assistant","timestamp":"2026-06-20T05:47:02Z","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_path","name":"Read","input":{"file_path":"src/export/nested_file.rs","workdir":"/repo/example_repo"}}]},"cwd":"/repo/example_repo"}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "--no-refresh",
            "special_file",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("src/export/special_file.rs"), "{stdout}");

    let nested = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "--no-refresh",
            "nested_file",
        ])
        .output()
        .unwrap();
    assert!(nested.status.success());
    let stdout = String::from_utf8(nested.stdout).unwrap();
    assert!(stdout.contains("nested_file"), "{stdout}");
}

#[test]
fn indexes_nested_claude_tool_result_errors() {
    let root = unique_temp_dir();
    let claude_home = root.join("claude");
    let transcripts = claude_home.join("transcripts");
    fs::create_dir_all(&transcripts).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        transcripts.join("ses_nested_tool.jsonl"),
        lines(&[
            r#"{"type":"user","timestamp":"2026-06-20T05:50:00Z","message":{"role":"user","content":"fix the build"},"cwd":"/repo/example_repo"}"#,
            r#"{"type":"assistant","timestamp":"2026-06-20T05:50:01Z","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"cargo test"}}]},"cwd":"/repo/example_repo"}"#,
            r#"{"type":"user","timestamp":"2026-06-20T05:50:02Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"error[E0425]: cannot find value `missing_symbol` in this scope"}]},"cwd":"/repo/example_repo"}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "--no-refresh",
            "missing_symbol",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("missing_symbol"), "{stdout}");
    assert!(stdout.contains("error_text"), "{stdout}");
}

#[test]
fn source_filter_applies_to_existing_mixed_indexes() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let claude_home = root.join("claude");
    let sessions = codex_home.join("sessions/2026/06/20");
    let transcripts = claude_home.join("transcripts");
    fs::create_dir_all(&sessions).unwrap();
    fs::create_dir_all(&transcripts).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        sessions.join("rollout-2026-06-20T06-00-00-99999999-9999-4999-8999-999999999999.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T06:00:00Z","type":"session_meta","payload":{"id":"99999999-9999-4999-8999-999999999999","cwd":"/repo/codex"}}"#,
            r#"{"timestamp":"2026-06-20T06:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"unique cross source needle"}}"#,
        ]),
    )
    .unwrap();
    fs::write(
        transcripts.join("ses_test.jsonl"),
        lines(&[r#"{"type":"user","timestamp":"2026-06-20T06:00:00Z","content":"isolated claude transcript marker","project":"/repo/claude"}"#]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let claude_only = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "--no-refresh",
            "unique cross source needle",
        ])
        .output()
        .unwrap();
    assert!(!claude_only.status.success());
    let stdout = String::from_utf8(claude_only.stdout).unwrap();
    assert!(stdout.contains("No matching sessions found"), "{stdout}");

    let codex_only = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "unique cross source needle",
        ])
        .output()
        .unwrap();
    assert!(codex_only.status.success());
    let stdout = String::from_utf8(codex_only.stdout).unwrap();
    assert!(stdout.contains("/repo/codex"), "{stdout}");

    let codex_excludes_claude = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "isolated claude transcript marker",
        ])
        .output()
        .unwrap();
    assert!(!codex_excludes_claude.status.success());
    let stdout = String::from_utf8(codex_excludes_claude.stdout).unwrap();
    assert!(stdout.contains("No matching sessions found"), "{stdout}");
}

#[test]
fn search_auto_indexes_selected_source_when_existing_docs_are_other_source() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let claude_home = root.join("claude");
    let sessions = codex_home.join("sessions/2026/06/20");
    let transcripts = claude_home.join("transcripts");
    fs::create_dir_all(&sessions).unwrap();
    fs::create_dir_all(&transcripts).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        sessions.join("rollout-2026-06-20T06-30-00-99999999-9999-4999-8999-999999999998.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T06:30:00Z","type":"session_meta","payload":{"id":"99999999-9999-4999-8999-999999999998","cwd":"/repo/codex"}}"#,
            r#"{"timestamp":"2026-06-20T06:30:01Z","type":"event_msg","payload":{"type":"user_message","message":"codex scoped auto index needle"}}"#,
        ]),
    )
    .unwrap();
    fs::write(
        transcripts.join("ses_later.jsonl"),
        lines(&[r#"{"type":"user","timestamp":"2026-06-20T06:31:00Z","content":"claude later auto refresh marker","project":"/repo/claude"}"#]),
    )
    .unwrap();

    let codex_first = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "codex scoped auto index needle",
        ])
        .output()
        .unwrap();
    assert!(codex_first.status.success());

    let claude_second = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "claude",
            "claude later auto refresh marker",
        ])
        .output()
        .unwrap();
    assert!(
        claude_second.status.success(),
        "{}",
        String::from_utf8_lossy(&claude_second.stdout)
    );
    let stdout = String::from_utf8(claude_second.stdout).unwrap();
    assert!(stdout.contains("claude:ses_later"), "{stdout}");
    assert!(
        stdout.contains("claude later auto refresh marker"),
        "{stdout}"
    );
}

#[test]
fn search_auto_indexes_missing_source_for_default_all() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let claude_home = root.join("claude");
    let sessions = codex_home.join("sessions/2026/06/20");
    let transcripts = claude_home.join("transcripts");
    fs::create_dir_all(&sessions).unwrap();
    fs::create_dir_all(&transcripts).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        sessions.join("rollout-2026-06-20T06-35-00-99999999-9999-4999-8999-999999999997.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T06:35:00Z","type":"session_meta","payload":{"id":"99999999-9999-4999-8999-999999999997","cwd":"/repo/codex"}}"#,
            r#"{"timestamp":"2026-06-20T06:35:01Z","type":"event_msg","payload":{"type":"user_message","message":"codex all-source seed needle"}}"#,
        ]),
    )
    .unwrap();
    fs::write(
        transcripts.join("ses_all_later.jsonl"),
        lines(&[r#"{"type":"user","timestamp":"2026-06-20T06:36:00Z","content":"claude default all refresh marker","project":"/repo/claude"}"#]),
    )
    .unwrap();

    let codex_first = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "codex all-source seed needle",
        ])
        .output()
        .unwrap();
    assert!(codex_first.status.success());

    let all_second = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--claude-home",
            claude_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "claude default all refresh marker",
        ])
        .output()
        .unwrap();
    assert!(
        all_second.status.success(),
        "{}",
        String::from_utf8_lossy(&all_second.stdout)
    );
    let stdout = String::from_utf8(all_second.stdout).unwrap();
    assert!(stdout.contains("claude:ses_all_later"), "{stdout}");
    assert!(
        stdout.contains("claude default all refresh marker"),
        "{stdout}"
    );
}

#[test]
fn migrates_legacy_sources_schema() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");
    fs::write(
        sessions.join("rollout-2026-06-20T07-00-00-aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T07:00:00Z","type":"session_meta","payload":{"id":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","cwd":"/repo/codex"}}"#,
            r#"{"timestamp":"2026-06-20T07:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"old schema migration needle"}}"#,
        ]),
    )
    .unwrap();
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        r#"
        create table sources (
            source_path text primary key,
            source_kind text not null,
            mtime_ns integer not null,
            size_bytes integer not null,
            indexed_at text not null
        );
        "#,
    )
    .unwrap();
    drop(conn);

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "old schema migration needle",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
}

#[test]
fn migrates_legacy_docs_fts_schema() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");
    fs::write(
        sessions.join("rollout-2026-06-20T07-30-00-aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaab.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T07:30:00Z","type":"session_meta","payload":{"id":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaab","cwd":"/repo/codex"}}"#,
            r#"{"timestamp":"2026-06-20T07:30:01Z","type":"event_msg","payload":{"type":"user_message","message":"legacy fts migration needle"}}"#,
        ]),
    )
    .unwrap();
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        r#"
        create table sessions (
            session_id text primary key,
            title text not null,
            cwd text not null,
            updated_at text not null,
            source_path text not null,
            repo text not null
        );
        create table docs (
            doc_id integer primary key,
            session_id text not null,
            source_kind text not null,
            source_path text not null,
            timestamp text not null,
            title text not null,
            cwd text not null,
            repo text not null,
            role text not null,
            category text not null,
            body text not null
        );
        create virtual table docs_fts using fts5(
            title,
            cwd,
            repo,
            role,
            category,
            body,
            tokenize='unicode61'
        );
        create table sources (
            source_path text primary key,
            mtime_ms integer not null,
            size_bytes integer not null
        );
        create table kv (
            key text primary key,
            value text not null
        );
        "#,
    )
    .unwrap();
    drop(conn);

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "legacy fts migration needle",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
}

#[test]
fn migrates_legacy_docs_fts_schema_without_repo_column() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");
    fs::write(
        sessions.join("rollout-2026-06-20T07-35-00-aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaad.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T07:35:00Z","type":"session_meta","payload":{"id":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaad","cwd":"/repo/codex"}}"#,
            r#"{"timestamp":"2026-06-20T07:35:01Z","type":"event_msg","payload":{"type":"user_message","message":"legacy fts repo migration needle"}}"#,
        ]),
    )
    .unwrap();
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        r#"
        create table sessions (
            session_id text primary key,
            title text not null,
            cwd text not null,
            updated_at text not null,
            source_path text not null,
            repo text not null
        );
        create table docs (
            doc_id integer primary key,
            session_id text not null,
            source_kind text not null,
            source_path text not null,
            timestamp text not null,
            title text not null,
            cwd text not null,
            repo text not null,
            role text not null,
            category text not null,
            body text not null
        );
        create virtual table docs_fts using fts5(
            title,
            cwd,
            role,
            category,
            body,
            source_path,
            tokenize='unicode61'
        );
        create table sources (
            source_path text primary key,
            mtime_ms integer not null,
            size_bytes integer not null
        );
        create table kv (
            key text primary key,
            value text not null
        );
        "#,
    )
    .unwrap();
    drop(conn);

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "legacy fts repo migration needle",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
}

#[test]
fn migrates_legacy_session_and_doc_repo_columns() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");
    fs::write(
        sessions.join("rollout-2026-06-20T07-40-00-aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaac.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T07:40:00Z","type":"session_meta","payload":{"id":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaac","cwd":"/repo/codex"}}"#,
            r#"{"timestamp":"2026-06-20T07:40:01Z","type":"event_msg","payload":{"type":"user_message","message":"legacy repo column migration needle"}}"#,
        ]),
    )
    .unwrap();
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        r#"
        create table sessions (
            session_id text primary key,
            title text not null,
            cwd text not null,
            updated_at text not null,
            source_path text not null
        );
        create table docs (
            doc_id integer primary key,
            session_id text not null,
            source_kind text not null,
            source_path text not null,
            timestamp text not null,
            title text not null,
            cwd text not null,
            role text not null,
            category text not null,
            body text not null
        );
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
        create table sources (
            source_path text primary key,
            mtime_ms integer not null,
            size_bytes integer not null
        );
        create table kv (
            key text primary key,
            value text not null
        );
        "#,
    )
    .unwrap();
    drop(conn);

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "legacy repo column migration needle",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
}

#[test]
fn indexes_common_error_only_tool_results() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        sessions.join("rollout-2026-06-20T08-00-00-bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T08:00:00Z","type":"session_meta","payload":{"id":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","cwd":"/repo/codex"}}"#,
            r#"{"timestamp":"2026-06-20T08:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"restore the database migration"}}"#,
            r#"{"timestamp":"2026-06-20T08:00:02Z","type":"response_item","payload":{"type":"function_call_output","output":"Error: No such file or directory: prisma/schema.prisma"}}"#,
        ]),
    )
    .unwrap();
    fs::write(
        sessions.join("rollout-2026-06-20T08-05-00-bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbc.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T08:05:00Z","type":"session_meta","payload":{"id":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbc","cwd":"/repo/codex"}}"#,
            r#"{"timestamp":"2026-06-20T08:05:01Z","type":"event_msg","payload":{"type":"user_message","message":"restore the database migration"}}"#,
            r#"{"timestamp":"2026-06-20T08:05:02Z","type":"response_item","payload":{"type":"function_call_output","output":"Exit code: 1\nOutput:\nplain migration failure without keywords"}}"#,
        ]),
    )
    .unwrap();
    fs::write(
        sessions.join("rollout-2026-06-20T08-10-00-bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbd.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T08:10:00Z","type":"session_meta","payload":{"id":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbd","cwd":"/repo/codex"}}"#,
            r#"{"timestamp":"2026-06-20T08:10:01Z","type":"event_msg","payload":{"type":"user_message","message":"debug command output indexing"}}"#,
            r#"{"timestamp":"2026-06-20T08:10:02Z","type":"event_msg","payload":{"type":"exec_command_end","exit_code":1,"aggregated_output":"No such file or directory: unique_missing_file_needle"}}"#,
        ]),
    )
    .unwrap();
    fs::write(
        sessions.join("rollout-2026-06-20T08-15-00-bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbe.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T08:15:00Z","type":"session_meta","payload":{"id":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbe","cwd":"/repo/codex"}}"#,
            r#"{"timestamp":"2026-06-20T08:15:01Z","type":"event_msg","payload":{"type":"user_message","message":"inspect successful command output"}}"#,
            r#"{"timestamp":"2026-06-20T08:15:02Z","type":"event_msg","payload":{"type":"exec_command_end","exit_code":0,"aggregated_output":"const message = 'error: not an actual command failure';"}}"#,
        ]),
    )
    .unwrap();
    fs::write(
        sessions.join("rollout-2026-06-20T08-20-00-bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbf.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T08:20:00Z","type":"session_meta","payload":{"id":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbf","cwd":"/repo/codex"}}"#,
            r#"{"timestamp":"2026-06-20T08:20:01Z","type":"event_msg","payload":{"type":"user_message","message":"inspect browser console"}}"#,
            r#"{"timestamp":"2026-06-20T08:20:02Z","type":"response_item","payload":{"type":"function_call_output","output":[{"type":"input_text","text":"Console: 1 error\nError: unique_array_error_needle failed"}]}}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "No such file",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("No such file"), "{stdout}");

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "plain migration failure",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("plain migration failure"), "{stdout}");

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "unique_missing_file_needle",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("unique_missing_file_needle"), "{stdout}");

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "actual command failure",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("No matching sessions found"), "{stdout}");

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "unique_array_error_needle",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("unique_array_error_needle"), "{stdout}");
}

#[test]
fn cwd_filter_restricts_rust_search_results() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        sessions.join("rollout-2026-06-20T09-30-00-dddddddd-dddd-4ddd-8ddd-dddddddddddd.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T09:30:00Z","type":"session_meta","payload":{"id":"dddddddd-dddd-4ddd-8ddd-dddddddddddd","cwd":"/repo/example_repo"}}"#,
            r#"{"timestamp":"2026-06-20T09:30:01Z","type":"event_msg","payload":{"type":"user_message","message":"restore database walkthrough"}}"#,
        ]),
    )
    .unwrap();
    fs::write(
        sessions.join("rollout-2026-06-20T09-31-00-eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T09:31:00Z","type":"session_meta","payload":{"id":"eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee","cwd":"/repo/other"}}"#,
            r#"{"timestamp":"2026-06-20T09:31:01Z","type":"event_msg","payload":{"type":"user_message","message":"restore database walkthrough example_repo"}}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "--cwd",
            "example_repo",
            "restore database",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("dddddddd-dddd-4ddd-8ddd-dddddddddddd"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee"),
        "{stdout}"
    );

    let cwd_only = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "--cwd",
            "example_repo",
        ])
        .output()
        .unwrap();
    assert!(cwd_only.status.success());
}

#[test]
fn cwd_filter_treats_sql_wildcards_as_literal_text() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        sessions.join("rollout-2026-06-20T09-40-00-dddddddd-dddd-4ddd-8ddd-ddddddddddde.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T09:40:00Z","type":"session_meta","payload":{"id":"dddddddd-dddd-4ddd-8ddd-ddddddddddde","cwd":"/repo/my_repo"}}"#,
            r#"{"timestamp":"2026-06-20T09:40:01Z","type":"event_msg","payload":{"type":"user_message","message":"shared cwd wildcard needle"}}"#,
        ]),
    )
    .unwrap();
    fs::write(
        sessions.join("rollout-2026-06-20T09-41-00-eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeef.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T09:41:00Z","type":"session_meta","payload":{"id":"eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeef","cwd":"/repo/myXrepo"}}"#,
            r#"{"timestamp":"2026-06-20T09:41:01Z","type":"event_msg","payload":{"type":"user_message","message":"shared cwd wildcard needle"}}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "--cwd",
            "my_repo",
            "shared cwd wildcard needle",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("dddddddd-dddd-4ddd-8ddd-ddddddddddde"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeef"),
        "{stdout}"
    );

    let cwd_only = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "--cwd",
            "my_repo",
        ])
        .output()
        .unwrap();
    assert!(cwd_only.status.success());
    let stdout = String::from_utf8(cwd_only.stdout).unwrap();
    assert!(
        stdout.contains("dddddddd-dddd-4ddd-8ddd-ddddddddddde"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeef"),
        "{stdout}"
    );
}

#[test]
fn clears_unversioned_indexes_before_reindexing() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");
    let subagent_path =
        sessions.join("rollout-2026-06-20T09-00-00-cccccccc-cccc-4ccc-8ccc-cccccccccccc.jsonl");
    fs::write(
        &subagent_path,
        lines(&[
            r#"{"timestamp":"2026-06-20T09:00:00Z","type":"session_meta","payload":{"id":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","parent_thread_id":"dddddddd-dddd-4ddd-8ddd-dddddddddddd","cwd":"/repo/example_repo","thread_source":"subagent","source":{"subagent":{"thread_spawn":{"depth":1}}}}}"#,
            r#"{"timestamp":"2026-06-20T09:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"stale subagent export ui phrase"}}"#,
        ]),
    )
    .unwrap();
    let stat = fs::metadata(&subagent_path).unwrap();
    let mtime_ms = stat
        .modified()
        .unwrap()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        r#"
        create table sessions (
            session_id text primary key,
            title text not null,
            cwd text not null,
            updated_at text not null,
            source_path text not null,
            session_kind text not null default 'full',
            parent_thread_id text not null default '',
            repo text not null
        );
        create table docs (
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
        create virtual table docs_fts using fts5(
            title, cwd, repo, role, category, body, source_path, tokenize='unicode61'
        );
        create table sources (
            source_path text primary key,
            mtime_ms integer not null,
            size_bytes integer not null
        );
        "#,
    )
    .unwrap();
    conn.execute(
        "insert into sessions(session_id,title,cwd,updated_at,source_path,session_kind,parent_thread_id,repo) values (?1,?2,?3,?4,?5,?6,?7,?8)",
        (
            "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
            "stale subagent",
            "/repo/example_repo",
            "2026-06-20T09:00:01Z",
            subagent_path.to_str().unwrap(),
            "full",
            "",
            "example_repo",
        ),
    )
    .unwrap();
    conn.execute(
        "insert into docs(session_id,source_kind,source_path,session_kind,timestamp,title,cwd,repo,role,category,body) values (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        (
            "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
            "codex-jsonl",
            subagent_path.to_str().unwrap(),
            "full",
            "2026-06-20T09:00:01Z",
            "stale subagent",
            "/repo/example_repo",
            "example_repo",
            "window",
            "conversation_window",
            "stale subagent export ui phrase",
        ),
    )
    .unwrap();
    let rowid = conn.last_insert_rowid();
    conn.execute(
        "insert into docs_fts(rowid,title,cwd,repo,role,category,body,source_path) values (?1,?2,?3,?4,?5,?6,?7,?8)",
        (
            rowid,
            "stale subagent",
            "/repo/example_repo",
            "example_repo",
            "window",
            "conversation_window",
            "stale subagent export ui phrase",
            subagent_path.to_str().unwrap(),
        ),
    )
    .unwrap();
    conn.execute(
        "insert into sources(source_path,mtime_ms,size_bytes) values (?1,?2,?3)",
        (subagent_path.to_str().unwrap(), mtime_ms, stat.len() as i64),
    )
    .unwrap();
    drop(conn);

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "stale subagent export ui phrase",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("No matching sessions found"), "{stdout}");
}

#[test]
fn orders_fts_candidates_before_search_cap() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");

    for index in 0..1250 {
        let id = format!("ffffffff-ffff-4fff-8fff-{index:012x}");
        let meta = format!(
            r#"{{"timestamp":"2026-06-20T10:00:00Z","type":"session_meta","payload":{{"id":"{id}","cwd":"/repo/noise"}}}}"#
        );
        fs::write(
            sessions.join(format!("rollout-2026-06-20T10-00-00-{id}.jsonl")),
            lines(&[
                meta.as_str(),
                r#"{"timestamp":"2026-06-20T10:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"ui only noise row"}}"#,
            ]),
        )
        .unwrap();
    }

    let wanted = "00000000-0000-4000-8000-000000000000";
    let wanted_meta = format!(
        r#"{{"timestamp":"2026-06-20T10:00:00Z","type":"session_meta","payload":{{"id":"{wanted}","cwd":"/repo/wanted"}}}}"#
    );
    fs::write(
        sessions.join(format!("rollout-2026-06-20T10-00-00-{wanted}.jsonl")),
        lines(&[
            wanted_meta.as_str(),
            r#"{"timestamp":"2026-06-20T10:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"needle export ui target session"}}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "--limit",
            "1",
            "needle export ui",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("needle export ui target session"),
        "{stdout}"
    );
    assert!(stdout.contains(wanted), "{stdout}");
}

#[test]
fn reindex_prunes_deleted_source_rows() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");
    let source =
        sessions.join("rollout-2026-06-20T10-30-00-12121212-1212-4212-8212-121212121212.jsonl");

    fs::write(
        &source,
        lines(&[
            r#"{"timestamp":"2026-06-20T10:30:00Z","type":"session_meta","payload":{"id":"12121212-1212-4212-8212-121212121212","cwd":"/repo"}}"#,
            r#"{"timestamp":"2026-06-20T10:30:01Z","type":"event_msg","payload":{"type":"user_message","message":"deleted stale needle"}}"#,
        ]),
    )
    .unwrap();

    for _ in 0..2 {
        let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
            .args([
                "--codex-home",
                codex_home.to_str().unwrap(),
                "--db",
                db.to_str().unwrap(),
                "--source",
                "codex",
                "index",
            ])
            .status()
            .unwrap();
        assert!(status.success());
        fs::remove_file(&source).unwrap_or(());
    }

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "deleted stale needle",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("No matching sessions found"), "{stdout}");

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "status",
        ])
        .output()
        .unwrap();
    assert!(status.status.success());
    let stdout = String::from_utf8(status.stdout).unwrap();
    assert!(stdout.contains("Sessions: 0"), "{stdout}");
    assert!(stdout.contains("Docs: 0"), "{stdout}");
    assert!(stdout.contains("Sources: 0"), "{stdout}");
}

#[test]
fn since_filter_restricts_existing_index_results() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let recent_ts = iso_for_epoch(now - 60);
    let old_ts = iso_for_epoch(now - 3 * 24 * 60 * 60);
    let old_meta = format!(
        r#"{{"timestamp":"{old_ts}","type":"session_meta","payload":{{"id":"11111111-1111-4111-8111-111111111111","cwd":"/repo"}}}}"#
    );
    let old_message = format!(
        r#"{{"timestamp":"{old_ts}","type":"event_msg","payload":{{"type":"user_message","message":"shared since needle ancient"}}}}"#
    );
    let recent_meta = format!(
        r#"{{"timestamp":"{recent_ts}","type":"session_meta","payload":{{"id":"22222222-2222-4222-8222-222222222222","cwd":"/repo"}}}}"#
    );
    let recent_message = format!(
        r#"{{"timestamp":"{recent_ts}","type":"event_msg","payload":{{"type":"user_message","message":"shared since needle recent"}}}}"#
    );

    fs::write(
        sessions.join("rollout-2026-06-20T11-00-00-11111111-1111-4111-8111-111111111111.jsonl"),
        lines(&[old_meta.as_str(), old_message.as_str()]),
    )
    .unwrap();
    fs::write(
        sessions.join("rollout-2026-06-20T11-01-00-22222222-2222-4222-8222-222222222222.jsonl"),
        lines(&[recent_meta.as_str(), recent_message.as_str()]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "--since",
            "1d",
            "shared since needle",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("recent"), "{stdout}");
    assert!(!stdout.contains("ancient"), "{stdout}");
}

#[test]
fn since_search_does_not_limit_the_persistent_index() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let recent_ts = iso_for_epoch(now - 60);
    let old_ts = iso_for_epoch(now - 3 * 24 * 60 * 60);
    let old_meta = format!(
        r#"{{"timestamp":"{old_ts}","type":"session_meta","payload":{{"id":"11111111-1111-4111-8111-111111111111","cwd":"/repo"}}}}"#
    );
    let old_message = format!(
        r#"{{"timestamp":"{old_ts}","type":"event_msg","payload":{{"type":"user_message","message":"persistent since needle ancient"}}}}"#
    );
    let recent_meta = format!(
        r#"{{"timestamp":"{recent_ts}","type":"session_meta","payload":{{"id":"22222222-2222-4222-8222-222222222222","cwd":"/repo"}}}}"#
    );
    let recent_message = format!(
        r#"{{"timestamp":"{recent_ts}","type":"event_msg","payload":{{"type":"user_message","message":"persistent since needle recent"}}}}"#
    );

    fs::write(
        sessions.join("rollout-2026-06-20T11-00-00-11111111-1111-4111-8111-111111111111.jsonl"),
        lines(&[old_meta.as_str(), old_message.as_str()]),
    )
    .unwrap();
    fs::write(
        sessions.join("rollout-2026-06-20T11-01-00-22222222-2222-4222-8222-222222222222.jsonl"),
        lines(&[recent_meta.as_str(), recent_message.as_str()]),
    )
    .unwrap();

    let recent = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--since",
            "1d",
            "persistent since needle",
        ])
        .output()
        .unwrap();
    assert!(recent.status.success());
    let stdout = String::from_utf8(recent.stdout).unwrap();
    assert!(stdout.contains("recent"), "{stdout}");
    assert!(!stdout.contains("ancient"), "{stdout}");

    let old = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "ancient",
        ])
        .output()
        .unwrap();
    assert!(old.status.success());
    let stdout = String::from_utf8(old.stdout).unwrap();
    assert!(stdout.contains("ancient"), "{stdout}");
}

#[test]
fn index_since_search_refreshes_recent_sources_when_docs_already_exist() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        sessions.join("rollout-2026-06-20T11-10-00-11111111-1111-4111-8111-111111111111.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T11:10:00Z","type":"session_meta","payload":{"id":"11111111-1111-4111-8111-111111111111","cwd":"/repo"}}"#,
            r#"{"timestamp":"2026-06-20T11:10:01Z","type":"event_msg","payload":{"type":"user_message","message":"existing indexed needle"}}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    fs::write(
        sessions.join("rollout-2026-06-20T11-11-00-22222222-2222-4222-8222-222222222222.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T11:11:00Z","type":"session_meta","payload":{"id":"22222222-2222-4222-8222-222222222222","cwd":"/repo"}}"#,
            r#"{"timestamp":"2026-06-20T11:11:01Z","type":"event_msg","payload":{"type":"user_message","message":"new index since refresh needle"}}"#,
        ]),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--index-since",
            "1d",
            "new index since refresh needle",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("new index since refresh needle"),
        "{stdout}"
    );
    assert!(
        stdout.contains("22222222-2222-4222-8222-222222222222"),
        "{stdout}"
    );
}

#[test]
fn no_archived_filter_restricts_existing_index_results() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let live_sessions = codex_home.join("sessions/2026/06/20");
    let archived_sessions = codex_home.join("archived_sessions/2026/06/20");
    fs::create_dir_all(&live_sessions).unwrap();
    fs::create_dir_all(&archived_sessions).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        live_sessions
            .join("rollout-2026-06-20T13-00-00-11111111-1111-4111-8111-111111111111.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T13:00:00Z","type":"session_meta","payload":{"id":"11111111-1111-4111-8111-111111111111","cwd":"/repo/live"}}"#,
            r#"{"timestamp":"2026-06-20T13:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"shared archive needle live session"}}"#,
        ]),
    )
    .unwrap();
    fs::write(
        archived_sessions
            .join("rollout-2026-06-20T13-01-00-22222222-2222-4222-8222-222222222222.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T13:01:00Z","type":"session_meta","payload":{"id":"22222222-2222-4222-8222-222222222222","cwd":"/repo/archived"}}"#,
            r#"{"timestamp":"2026-06-20T13:01:01Z","type":"event_msg","payload":{"type":"user_message","message":"shared archive needle archived session"}}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "--no-archived",
            "shared archive needle",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("11111111-1111-4111-8111-111111111111"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("22222222-2222-4222-8222-222222222222"),
        "{stdout}"
    );
    assert!(stdout.contains("live session"), "{stdout}");
    assert!(!stdout.contains("archived session"), "{stdout}");
}

#[test]
fn no_archived_index_does_not_prune_archived_rows() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let live_sessions = codex_home.join("sessions/2026/06/20");
    let archived_sessions = codex_home.join("archived_sessions/2026/06/20");
    fs::create_dir_all(&live_sessions).unwrap();
    fs::create_dir_all(&archived_sessions).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        live_sessions
            .join("rollout-2026-06-20T13-10-00-11111111-1111-4111-8111-111111111111.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T13:10:00Z","type":"session_meta","payload":{"id":"11111111-1111-4111-8111-111111111111","cwd":"/repo/live"}}"#,
            r#"{"timestamp":"2026-06-20T13:10:01Z","type":"event_msg","payload":{"type":"user_message","message":"persistent archive needle live session"}}"#,
        ]),
    )
    .unwrap();
    fs::write(
        archived_sessions
            .join("rollout-2026-06-20T13-11-00-22222222-2222-4222-8222-222222222222.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T13:11:00Z","type":"session_meta","payload":{"id":"22222222-2222-4222-8222-222222222222","cwd":"/repo/archived"}}"#,
            r#"{"timestamp":"2026-06-20T13:11:01Z","type":"event_msg","payload":{"type":"user_message","message":"persistent archive needle archived session"}}"#,
        ]),
    )
    .unwrap();

    for extra_args in [Vec::<&str>::new(), vec!["--no-archived"]] {
        let mut args = vec![
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
        ];
        args.extend(extra_args);
        args.push("index");
        let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
    }

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "archived session",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("22222222-2222-4222-8222-222222222222"),
        "{stdout}"
    );
    assert!(stdout.contains("archived session"), "{stdout}");
}

#[test]
fn no_archived_index_skips_unreadable_archived_tree() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let live_sessions = codex_home.join("sessions/2026/06/20");
    let unreadable_archived = codex_home.join("archived_sessions/private");
    fs::create_dir_all(&live_sessions).unwrap();
    fs::create_dir_all(&unreadable_archived).unwrap();
    fs::set_permissions(&unreadable_archived, fs::Permissions::from_mode(0o000)).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        live_sessions
            .join("rollout-2026-06-20T13-20-00-11111111-1111-4111-8111-111111111111.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T13:20:00Z","type":"session_meta","payload":{"id":"11111111-1111-4111-8111-111111111111","cwd":"/repo/live"}}"#,
            r#"{"timestamp":"2026-06-20T13:20:01Z","type":"event_msg","payload":{"type":"user_message","message":"unreadable archive skip needle"}}"#,
        ]),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-archived",
            "index",
        ])
        .output()
        .unwrap();
    fs::set_permissions(&unreadable_archived, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "unreadable archive skip needle",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("unreadable archive skip needle"),
        "{stdout}"
    );
}

#[test]
fn invalid_duration_arguments_fail() {
    for (flag, value) in [
        ("--since", "1day"),
        ("--index-since", "recent"),
        ("--since", "日"),
        ("--index-since", "１d"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
            .args([flag, value, "export bugs"])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{flag} {value}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("invalid duration"), "{stderr}");
    }
}

#[test]
fn nanosecond_mtime_changes_force_same_size_reindex() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");
    let source =
        sessions.join("rollout-2026-06-20T13-30-00-33333333-3333-4333-8333-333333333333.jsonl");
    let old_body = lines(&[
        r#"{"timestamp":"2026-06-20T13:30:00Z","type":"session_meta","payload":{"id":"33333333-3333-4333-8333-333333333333","cwd":"/repo"}}"#,
        r#"{"timestamp":"2026-06-20T13:30:01Z","type":"event_msg","payload":{"type":"user_message","message":"same size oldtoken phrase"}}"#,
    ]);
    let new_body = lines(&[
        r#"{"timestamp":"2026-06-20T13:30:00Z","type":"session_meta","payload":{"id":"33333333-3333-4333-8333-333333333333","cwd":"/repo"}}"#,
        r#"{"timestamp":"2026-06-20T13:30:01Z","type":"event_msg","payload":{"type":"user_message","message":"same size newtoken phrase"}}"#,
    ]);
    assert_eq!(old_body.len(), new_body.len());
    fs::write(&source, old_body).unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    fs::write(&source, new_body).unwrap();
    let stat = fs::metadata(&source).unwrap();
    let modified = stat.modified().unwrap().duration_since(UNIX_EPOCH).unwrap();
    let mtime_ns = modified.as_nanos().min(i64::MAX as u128) as i64;
    let mtime_ms = modified.as_millis() as i64;
    let conn = Connection::open(&db).unwrap();
    conn.execute(
        "update sources set mtime_ns=?1, mtime_ms=?2, size_bytes=?3 where source_path=?4",
        (
            mtime_ns.saturating_sub(1),
            mtime_ms,
            stat.len() as i64,
            source.to_str().unwrap(),
        ),
    )
    .unwrap();
    drop(conn);

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let updated = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "newtoken",
        ])
        .output()
        .unwrap();
    assert!(updated.status.success());
    let stdout = String::from_utf8(updated.stdout).unwrap();
    assert!(stdout.contains("newtoken"), "{stdout}");

    let stale = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "--no-refresh",
            "oldtoken",
        ])
        .output()
        .unwrap();
    assert!(!stale.status.success());
}

#[test]
fn reindex_prunes_source_that_becomes_subagent() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");
    let source =
        sessions.join("rollout-2026-06-20T13-45-00-44444444-4444-4444-8444-444444444444.jsonl");

    fs::write(
        &source,
        lines(&[
            r#"{"timestamp":"2026-06-20T13:45:00Z","type":"session_meta","payload":{"id":"44444444-4444-4444-8444-444444444444","cwd":"/repo","thread_source":"user"}}"#,
            r#"{"timestamp":"2026-06-20T13:45:01Z","type":"event_msg","payload":{"type":"user_message","message":"stale full session needle"}}"#,
        ]),
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    fs::write(
        &source,
        lines(&[
            r#"{"timestamp":"2026-06-20T13:45:00Z","type":"session_meta","payload":{"id":"44444444-4444-4444-8444-444444444444","parent_thread_id":"55555555-5555-4555-8555-555555555555","cwd":"/repo","thread_source":"subagent"}}"#,
            r#"{"timestamp":"2026-06-20T13:45:01Z","type":"event_msg","payload":{"type":"user_message","message":"new worker-only needle"}}"#,
        ]),
    )
    .unwrap();
    let stat = fs::metadata(&source).unwrap();
    let modified = stat.modified().unwrap().duration_since(UNIX_EPOCH).unwrap();
    let mtime_ns = modified.as_nanos().min(i64::MAX as u128) as i64;
    let mtime_ms = modified.as_millis() as i64;
    let conn = Connection::open(&db).unwrap();
    conn.execute(
        "update sources set mtime_ns=?1, mtime_ms=?2, size_bytes=?3 where source_path=?4",
        (
            mtime_ns.saturating_sub(1),
            mtime_ms,
            stat.len() as i64,
            source.to_str().unwrap(),
        ),
    )
    .unwrap();
    drop(conn);

    let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
        .args([
            "--codex-home",
            codex_home.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--source",
            "codex",
            "index",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    for query in ["stale full session needle", "new worker-only needle"] {
        let output = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
            .args([
                "--codex-home",
                codex_home.to_str().unwrap(),
                "--db",
                db.to_str().unwrap(),
                "--source",
                "codex",
                "--no-refresh",
                query,
            ])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("No matching sessions found"), "{stdout}");
    }
}

#[test]
fn max_sources_advances_past_unchanged_sources() {
    let root = unique_temp_dir();
    let codex_home = root.join("codex");
    let sessions = codex_home.join("sessions/2026/06/20");
    fs::create_dir_all(&sessions).unwrap();
    let db = root.join("index.sqlite");

    fs::write(
        sessions.join("rollout-2026-06-20T12-00-00-11111111-1111-4111-8111-111111111111.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T12:00:00Z","type":"session_meta","payload":{"id":"11111111-1111-4111-8111-111111111111","cwd":"/repo"}}"#,
            r#"{"timestamp":"2026-06-20T12:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"newest bounded index needle"}}"#,
        ]),
    )
    .unwrap();
    fs::write(
        sessions.join("rollout-2026-06-20T11-59-00-22222222-2222-4222-8222-222222222222.jsonl"),
        lines(&[
            r#"{"timestamp":"2026-06-20T11:59:00Z","type":"session_meta","payload":{"id":"22222222-2222-4222-8222-222222222222","cwd":"/repo"}}"#,
            r#"{"timestamp":"2026-06-20T11:59:01Z","type":"event_msg","payload":{"type":"user_message","message":"older bounded index needle"}}"#,
        ]),
    )
    .unwrap();

    for _ in 0..2 {
        let status = Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
            .args([
                "--codex-home",
                codex_home.to_str().unwrap(),
                "--db",
                db.to_str().unwrap(),
                "--source",
                "codex",
                "--max-sources",
                "1",
                "index",
            ])
            .status()
            .unwrap();
        assert!(status.success());
    }

    let conn = Connection::open(&db).unwrap();
    let sessions: i64 = conn
        .query_row("select count(*) from sessions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(sessions, 2);
}

fn lines(rows: &[&str]) -> String {
    let mut body = rows.join("\n");
    body.push('\n');
    body
}

fn unique_temp_dir() -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "agent-session-finder-test-{}-{nanos}-{counter}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn iso_for_epoch(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let second_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = second_of_day / 3_600;
    let minute = second_of_day % 3_600 / 60;
    let second = second_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    (year, month, day)
}
