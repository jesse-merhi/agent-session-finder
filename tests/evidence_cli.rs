use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "agent-evidence-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_agent-session-find"))
            .current_dir(&self.0)
            .env(
                "AGENT_SESSION_FINDER_DB",
                self.0.join("must-not-create/index.sqlite"),
            )
            .args(args)
            .output()
            .unwrap()
    }

    fn page(&self, args: &[&str], budget: usize) -> Value {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stdout.len() <= budget,
            "{} bytes",
            output.stdout.len()
        );
        assert!(!self.0.join("must-not-create").exists());
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn reads_late_matches_and_both_codex_tool_formats_with_source_lines() {
    let fixture = Fixture::new();
    let long_text = format!("{}needle at the end", "界🙂".repeat(10_000));
    let rows = [
        json!({"type":"session_meta","payload":{"instructions":"needle must stay private"}}),
        json!({"type":"response_item","payload":{"type":"message","role":"developer","content":[{"text":"needle instruction"}]}}),
        json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":long_text}]}}),
        json!({"type":"response_item","payload":{"type":"function_call_output","output":"needle legacy output"}}),
        json!({"type":"response_item","payload":{"type":"custom_tool_call_output","output":[{"type":"input_text","text":"needle current output"},{"type":"image","data":"needle image bytes"}]}}),
        json!({"type":"response_item","payload":{"type":"function_call","name":"exec","arguments":"needle command"}}),
        json!({"type":"response_item","payload":{"type":"custom_tool_call","name":"exec","input":"needle custom command"}}),
    ];
    let original = rows
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(fixture.0.join("session.jsonl"), &original).unwrap();
    let first = fixture.page(&["--read", "session.jsonl", "needle", "--limit", "2"], 8192);
    assert_eq!(first["items"][0]["line"], 3);
    assert_eq!(first["items"][0]["kind"], "assistant");
    assert!(first["items"][0]["text"]
        .as_str()
        .unwrap()
        .contains("needle at the end"));
    assert!(first["items"][0]["text_start"].as_u64().unwrap() > 60_000);
    assert_eq!(first["items"][0]["truncated"], true);
    assert_eq!(first["next_offset"], 2);
    let next = fixture.page(
        &["--read", "session.jsonl", "needle", "--offset", "2"],
        8192,
    );
    assert_eq!(next["items"].as_array().unwrap().len(), 3);
    assert_eq!(next["items"][0]["line"], 5);
    assert_eq!(next["items"][0]["text"], "needle current output");
    assert_eq!(next["items"][1]["kind"], "tool_call");
    assert_eq!(next["items"][2]["line"], 7);
    assert!(next["next_offset"].is_null());
    assert_eq!(
        fs::read_to_string(fixture.0.join("session.jsonl")).unwrap(),
        original
    );
}

#[test]
fn pages_by_serialized_bytes_without_losing_escaped_unicode_passages() {
    let fixture = Fixture::new();
    let text = (0..12)
        .map(|i| format!("needle-{i}{}", "界\n\t\"\\".repeat(200)))
        .collect::<String>();
    fs::write(fixture.0.join("session.jsonl"), json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":text}]}}).to_string()).unwrap();
    let mut offset = 0;
    let mut found = Vec::new();
    loop {
        let page = fixture.page(
            &[
                "--read",
                "session.jsonl",
                "needle",
                "--offset",
                &offset.to_string(),
                "--max-bytes",
                "2048",
            ],
            2048,
        );
        for item in page["items"].as_array().unwrap() {
            let start = item["text_start"].as_u64().unwrap() as usize;
            let end = item["text_end"].as_u64().unwrap() as usize;
            assert_eq!(item["text"], &text[start..end]);
            found.push(item["text"].as_str().unwrap().to_string());
        }
        match page["next_offset"].as_u64() {
            Some(next) => {
                assert!(next > offset);
                offset = next;
            }
            None => break,
        }
        assert!(offset < 20);
    }
    assert_eq!(found.len(), 12);
    for (i, passage) in found.iter().enumerate() {
        assert!(passage.contains(&format!("needle-{i}")));
    }
}

#[test]
fn reads_claude_text_and_nested_tools_and_legacy_codex_events() {
    let fixture = Fixture::new();
    let rows = [
        json!({"type":"assistant","message":{"content":[{"type":"text","text":"needle explanation"},{"type":"tool_use","name":"Bash","input":{"command":"needle command"}}]}}),
        json!({"type":"user","message":{"content":[{"type":"tool_result","content":[{"type":"text","text":"needle result"}]}]}}),
        json!({"type":"event_msg","payload":{"type":"user_message","message":"needle old event"}}),
        json!({"type":"event_msg","payload":{"type":"agent_message","message":"needle old reply"}}),
    ];
    fs::write(
        fixture.0.join("session.jsonl"),
        rows.iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let page = fixture.page(&["--read", "session.jsonl", "needle"], 8192);
    let kinds: Vec<_> = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        ["assistant", "tool_call", "tool_output", "user", "assistant"]
    );
}

#[test]
fn compacts_large_search_hits_from_raw_and_mcp_envelopes_and_preserves_urls() {
    let fixture = Fixture::new();
    let hits: Vec<_> = (0..3)
        .map(|i| {
            json!({
                "url":format!("https://developers.openai.com/example/{i}#section"),
                "hierarchy":{"lvl0":"Docs","lvl1":format!("Page {i}")},
                "content":"full page ".repeat(6000),
                "_snippetResult":{"content":{"value":format!("matching excerpt {i}")}},
                "_highlightResult":{"content":{"value":"duplicate page ".repeat(6000)}}
            })
        })
        .collect();
    let data = json!({"hits":hits,"page":2,"nbHits":30,"nextCursor":"provider-cursor"});
    for input in [
        data.clone(),
        json!({"content":[{"type":"text","text":data.to_string()}]}),
        json!({"structuredContent":data}),
    ] {
        let original = input.to_string();
        fs::write(fixture.0.join("search.json"), &original).unwrap();
        let page = fixture.page(
            &["--compact-docs", "search.json", "--max-bytes", "2048"],
            2048,
        );
        assert_eq!(page["items"].as_array().unwrap().len(), 3);
        for i in 0..3 {
            assert_eq!(page["items"][i]["url"], hits[i]["url"]);
            assert_eq!(page["items"][i]["excerpt"], format!("matching excerpt {i}"));
        }
        assert_eq!(page["search"]["next_cursor"], "provider-cursor");
        assert_eq!(page["search"]["total_hits"], 30);
        assert_eq!(page["search"]["page"], 2);
        assert!(page["next_offset"].is_null());
        assert!(!page.to_string().contains("duplicate page"));
        assert_eq!(
            fs::read_to_string(fixture.0.join("search.json")).unwrap(),
            original
        );
    }
}

#[test]
fn caps_each_document_excerpt_and_resumes_supplied_hits_separately_from_search_pages() {
    let fixture = Fixture::new();
    let hits: Vec<_> = (0..5)
        .map(|i| json!({"url":format!("https://example.org/{i}"),"content":"🙂".repeat(1000)}))
        .collect();
    fs::write(
        fixture.0.join("search.json"),
        json!({"hits":hits}).to_string(),
    )
    .unwrap();
    let first = fixture.page(&["--compact-docs", "search.json", "--limit", "2"], 8192);
    assert_eq!(first["next_offset"], 2);
    assert_eq!(first["items"][0]["excerpt"].as_str().unwrap().len(), 240);
    assert_eq!(first["items"][0]["truncated"], true);
    let next = fixture.page(&["--compact-docs", "search.json", "--offset", "2"], 8192);
    assert_eq!(next["items"][0]["hit"], 2);
    assert_eq!(next["items"].as_array().unwrap().len(), 3);
    assert!(next["next_offset"].is_null());
}

#[test]
fn rejects_invalid_inputs_without_partial_output_or_index_writes() {
    let fixture = Fixture::new();
    for (filename, input, args, expected) in [
        (
            "bad.jsonl",
            "not json".to_string(),
            vec!["--read", "bad.jsonl"],
            "source line 1",
        ),
        (
            "error.json",
            json!({"isError":true,"content":[{"type":"text","text":"failure"}]}).to_string(),
            vec!["--compact-docs", "error.json"],
            "MCP error",
        ),
        (
            "shape.json",
            "{}".to_string(),
            vec!["--compact-docs", "shape.json"],
            "expected search JSON",
        ),
        (
            "hit.json",
            json!({"hits":[{"content":"no URL"}]}).to_string(),
            vec!["--compact-docs", "hit.json"],
            "no URL",
        ),
        (
            "url.json",
            json!({"hits":[{"url":format!("https://example.org/{}", "x".repeat(3000))}]})
                .to_string(),
            vec!["--compact-docs", "url.json", "--max-bytes", "1024"],
            "increase the budget",
        ),
        (
            "empty.jsonl",
            String::new(),
            vec!["--read", "empty.jsonl", "--max-bytes", "0"],
            "at least 1024",
        ),
        (
            "both.json",
            "{}".to_string(),
            vec!["--read", "both.json", "--compact-docs", "both.json"],
            "exactly one",
        ),
    ] {
        fs::write(fixture.0.join(filename), input).unwrap();
        let result = fixture.run(&args);
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&result.stderr).contains(expected),
            "{:?}",
            result
        );
        assert!(!fixture.0.join("must-not-create").exists());
    }
}

#[test]
fn retrieves_supported_tool_records_from_actual_search_result_paths() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.0.join("codex/sessions")).unwrap();
    fs::create_dir_all(fixture.0.join("claude/transcripts")).unwrap();
    let codex = [
        json!({"type":"session_meta","payload":{"id":"11111111-1111-4111-8111-111111111111","cwd":"/example"}}),
        json!({"type":"event_msg","payload":{"type":"user_message","message":"Run the build"}}),
        json!({"type":"event_msg","payload":{"type":"exec_command_end","exit_code":1,"aggregated_output":"No such file unique_missing_file_needle"}}),
    ];
    let claude = [
        json!({"type":"user","content":"Find the route","project":"/example"}),
        json!({"type":"tool_use","tool_name":"grep","tool_input":{"file_path":"src/special_file.rs"}}),
        json!({"type":"tool_result","tool_output":{"stderr":"No such file flat_result_needle"}}),
    ];
    for (name, rows) in [
        ("codex/sessions/run.jsonl", codex.as_slice()),
        ("claude/transcripts/run.jsonl", claude.as_slice()),
    ] {
        fs::write(
            fixture.0.join(name),
            rows.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
    }
    let indexed = fixture.run(&[
        "--codex-home",
        "codex",
        "--claude-home",
        "claude",
        "--db",
        "index.sqlite",
        "index",
    ]);
    assert!(
        indexed.status.success(),
        "{}",
        String::from_utf8_lossy(&indexed.stderr)
    );
    for needle in [
        "unique_missing_file_needle",
        "special_file",
        "flat_result_needle",
    ] {
        let search = fixture.run(&["--db", "index.sqlite", "--no-refresh", needle]);
        assert!(
            search.status.success(),
            "{}",
            String::from_utf8_lossy(&search.stderr)
        );
        let output = String::from_utf8(search.stdout).unwrap();
        let source = output
            .lines()
            .find_map(|line| line.trim().strip_prefix("src: "))
            .unwrap();
        let page = fixture.page(&["--read", source, needle], 8192);
        assert!(
            page["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["text"].as_str().unwrap().contains(needle)),
            "{needle}: {page}"
        );
    }
}

#[test]
fn retains_a_literal_crossing_the_previous_excerpt_boundary() {
    let fixture = Fixture::new();
    let needle = "requirements.toml";
    let text = format!("{needle}{}{needle}", " ".repeat(470 - needle.len()));
    fs::write(fixture.0.join("session.jsonl"), json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":text}]}}).to_string()).unwrap();
    let first = fixture.page(&["--read", "session.jsonl", needle, "--limit", "1"], 8192);
    assert_eq!(first["next_offset"], 1);
    let next = fixture.page(&["--read", "session.jsonl", needle, "--offset", "1"], 8192);
    assert_eq!(next["items"].as_array().unwrap().len(), 1);
    assert!(next["items"][0]["text"].as_str().unwrap().contains(needle));
    assert_eq!(next["items"][0]["text_end"], text.len());
    assert!(next["next_offset"].is_null());
}

#[test]
fn treats_help_and_version_after_delimiter_as_literal_queries() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("session.jsonl"), json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"cargo --help --version -h -V"}]}}).to_string()).unwrap();
    for query in ["--help", "--version", "-h", "-V"] {
        let page = fixture.page(
            &[
                "--read",
                "session.jsonl",
                "--max-bytes",
                "1024",
                "--",
                query,
            ],
            1024,
        );
        assert!(page["items"][0]["text"].as_str().unwrap().contains(query));
    }
}

#[test]
fn matches_decoded_quotes_and_newlines_in_tool_inputs() {
    let fixture = Fixture::new();
    let command = "grep -F \"requirements.toml\" Cargo.toml\nprintf 'done'";
    let input = json!({"cmd":command,"nested":{"argv":[command]}});
    let rows = [
        json!({"type":"response_item","payload":{"type":"function_call","name":"exec","arguments":input.to_string()}}),
        json!({"type":"response_item","payload":{"type":"custom_tool_call","name":"exec","input":command}}),
        json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":command}}]}}),
        json!({"type":"tool_use","tool_name":"Bash","tool_input":{"command":command}}),
    ];
    fs::write(
        fixture.0.join("session.jsonl"),
        rows.iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let page = fixture.page(&["--read", "session.jsonl", command], 8192);
    let items = page["items"].as_array().unwrap();
    assert_eq!(items.len(), 4);
    assert!(items
        .iter()
        .all(|item| item["text"].as_str().unwrap().contains(command)));
}

#[test]
fn reduces_unicode_context_to_keep_fitting_literals_complete() {
    let fixture = Fixture::new();
    for bytes in [385, 448, 479, 480] {
        let needle = "x".repeat(bytes);
        let text = format!("{}{needle} suffix", "界🙂".repeat(30));
        fs::write(fixture.0.join("session.jsonl"), json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"text":text}]}}).to_string()).unwrap();
        let page = fixture.page(&["--read", "session.jsonl", &needle], 8192);
        let excerpt = page["items"][0]["text"].as_str().unwrap();
        assert!(excerpt.contains(&needle), "{bytes}: {page}");
        assert!(excerpt.len() <= 480);
        assert_eq!(page["items"].as_array().unwrap().len(), 1);
    }
}

#[test]
fn reads_native_search_arguments_actions_and_discovered_tools() {
    let fixture = Fixture::new();
    // Field shapes follow Codex protocol models.rs ResponseItem variants.
    let payloads = [
        json!({"type":"tool_search_call","call_id":"search-1","execution":"client","arguments":{"query":"calendar create","limit":1}}),
        json!({"type":"web_search_call","status":"completed","action":{"type":"search","queries":["calendar documentation","calendar examples"]}}),
        json!({"type":"web_search_call","status":"completed","action":{"type":"open_page","url":"https://example.com/calendar"}}),
        json!({"type":"web_search_call","status":"completed","action":{"type":"find_in_page","url":"https://example.com","pattern":"calendar settings"}}),
        json!({"type":"tool_search_output","call_id":"search-1","status":"completed","execution":"client","tools":[{"type":"function","name":"calendar_create_event","description":"Create a calendar event.","parameters":{"type":"object","properties":{"title":{"type":"string"}}}}]}),
    ];
    fs::write(
        fixture.0.join("session.jsonl"),
        payloads
            .iter()
            .map(|payload| json!({"type":"response_item","payload":payload}).to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let page = fixture.page(&["--read", "session.jsonl", "calendar"], 8192);
    assert_eq!(page["items"].as_array().unwrap().len(), 5);
    for (i, expected) in [
        "calendar create",
        "calendar documentation",
        "https://example.com/calendar",
        "calendar settings",
        "calendar_create_event",
    ]
    .iter()
    .enumerate()
    {
        assert!(page["items"][i]["text"]
            .as_str()
            .unwrap()
            .contains(expected));
        assert_eq!(page["items"][i]["line"], i + 1);
    }
}

#[test]
fn reads_wrapped_and_current_tool_output_as_verbatim_text() {
    let fixture = Fixture::new();
    let text = "\n  Success. Updated \"file.txt\":\n  M file.txt\n";
    let outputs = [
        json!(json!({"output":text,"metadata":{"exit_code":0,"duration_seconds":0.1}}).to_string()),
        json!({"stderr":text}),
        json!({"output":"","metadata":{"stderr":text}}),
        json!([{ "type":"input_text", "text":text }]),
        json!(text),
        json!(json!({"content":[{"type":"text","text":text}]}).to_string()),
    ];
    for kind in ["function_call_output", "custom_tool_call_output"] {
        for output in &outputs {
            fs::write(
                fixture.0.join("session.jsonl"),
                json!({"type":"response_item","payload":{"type":kind,"output":output}}).to_string(),
            )
            .unwrap();
            let page = fixture.page(&["--read", "session.jsonl", text], 8192);
            assert_eq!(
                page["items"].as_array().unwrap().len(),
                1,
                "{kind}: {output}"
            );
            assert_eq!(page["items"][0]["text"], text);
            assert_eq!(page["items"][0]["text_bytes"], text.len());
        }
    }
}

#[test]
fn keeps_plain_json_tool_output_when_it_is_not_a_text_envelope() {
    let fixture = Fixture::new();
    for text in [r#"[1,2,"value"]"#, r#"{"content":[1,2]}"#, r#"{"count":3}"#] {
        fs::write(
            fixture.0.join("session.jsonl"),
            json!({"type":"response_item","payload":{"type":"function_call_output","output":text}})
                .to_string(),
        )
        .unwrap();
        let page = fixture.page(&["--read", "session.jsonl", text], 8192);
        assert_eq!(page["items"][0]["text"], text);
    }
}
