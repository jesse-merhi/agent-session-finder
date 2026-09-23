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
