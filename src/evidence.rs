use crate::{content_text, previous_char_boundary, required_arg, trim, write_io, AppResult};
use serde_json::{json, Value};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;

#[derive(Clone, Copy)]
enum Kind {
    Transcript,
    Docs,
}

struct Options {
    path: PathBuf,
    kind: Kind,
    query: String,
    max_bytes: usize,
    offset: usize,
    limit: usize,
}

pub fn run_if_requested(args: &[String]) -> AppResult<bool> {
    if !args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--read" | "--compact-docs"))
    {
        return Ok(false);
    }
    let options = parse_options(args)?;
    let mut page = Page::new(&options);
    match options.kind {
        Kind::Transcript => read_transcript(&options, &mut page)?,
        Kind::Docs => read_docs(&options, &mut page)?,
    }
    let output = page.serialize();
    if output.len() + 1 > options.max_bytes {
        return Err("response metadata exceeds --max-bytes; increase the budget".into());
    }
    write_io(writeln!(io::stdout().lock(), "{output}"))?;
    Ok(true)
}

fn parse_options(args: &[String]) -> AppResult<Options> {
    let mut input = None;
    let mut query = Vec::new();
    let mut max_bytes = 8192;
    let mut offset = 0;
    let mut limit = 10;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--read" | "--compact-docs" => {
                if input.is_some() {
                    return Err("choose exactly one --read or --compact-docs input".into());
                }
                let kind = if args[index] == "--read" {
                    Kind::Transcript
                } else {
                    Kind::Docs
                };
                index += 1;
                input = Some((
                    crate::expand_home(&required_arg(args, index, "input")?),
                    kind,
                ));
            }
            "--max-bytes" | "--offset" | "--limit" => {
                let flag = &args[index];
                index += 1;
                let value: usize = required_arg(args, index, flag)?
                    .parse()
                    .map_err(|_| format!("{flag} needs a nonnegative integer"))?;
                match flag.as_str() {
                    "--max-bytes" => max_bytes = value,
                    "--offset" => offset = value,
                    _ => limit = value,
                }
            }
            "--" => {
                query.extend_from_slice(&args[index + 1..]);
                break;
            }
            arg if arg.starts_with('-') => {
                return Err(format!("unsupported read option: {arg}; run --help").into())
            }
            arg => query.push(arg.to_string()),
        }
        index += 1;
    }
    let (path, kind) = input.ok_or("--read or --compact-docs needs a path")?;
    if max_bytes < 1024 || limit == 0 {
        return Err(
            "--max-bytes must be at least 1024 and --limit must be greater than zero".into(),
        );
    }
    if matches!(kind, Kind::Docs) && !query.is_empty() {
        return Err(
            "--compact-docs projects the supplied hits; queries are only supported with --read"
                .into(),
        );
    }
    Ok(Options {
        path,
        kind,
        query: query.join(" "),
        max_bytes,
        offset,
        limit,
    })
}

struct Page<'a> {
    options: &'a Options,
    items: Vec<Value>,
    seen: usize,
    next_offset: Option<usize>,
    search: Value,
}

impl<'a> Page<'a> {
    fn new(options: &'a Options) -> Self {
        Self {
            options,
            items: Vec::new(),
            seen: 0,
            next_offset: None,
            search: Value::Null,
        }
    }

    fn serialize(&self) -> String {
        json!({
            "source": self.options.path,
            "items": self.items,
            "next_offset": self.next_offset,
            "search": self.search,
        })
        .to_string()
    }

    // Reserve space for a continuation before accepting any item. The budget
    // includes JSON escaping, source metadata, and the terminating newline.
    fn push(&mut self, item: Value) -> AppResult<bool> {
        if self.seen < self.options.offset {
            self.seen += 1;
            return Ok(true);
        }
        self.next_offset = Some(usize::MAX);
        self.items.push(item);
        let fits = self.items.len() <= self.options.limit
            && self.serialize().len() < self.options.max_bytes;
        if !fits {
            self.items.pop();
            if self.items.is_empty() {
                return Err("one evidence item exceeds --max-bytes; increase the budget to preserve its source reference".into());
            }
            self.next_offset = Some(self.seen);
            return Ok(false);
        }
        self.seen += 1;
        self.next_offset = None;
        Ok(true)
    }
}

fn read_transcript(options: &Options, page: &mut Page<'_>) -> AppResult<()> {
    let input = BufReader::new(File::open(&options.path)?);
    for (line_index, line) in input.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let row: Value = serde_json::from_str(&line)
            .map_err(|err| format!("invalid JSON at source line {}: {err}", line_index + 1))?;
        for (kind, text) in transcript_text(&row, &options.query) {
            if !add_passages(page, line_index + 1, kind, &text, &options.query)? {
                return Ok(());
            }
        }
    }
    Ok(())
}

fn transcript_text(row: &Value, query: &str) -> Vec<(&'static str, String)> {
    let payload = &row["payload"];
    if row["type"] == "response_item" {
        let entry = match payload["type"].as_str().unwrap_or_default() {
            "message" => match payload["role"].as_str() {
                Some("user") => ("user", content_text(&payload["content"])),
                Some("assistant") => ("assistant", content_text(&payload["content"])),
                _ => return Vec::new(),
            },
            "agent_message" => ("agent_message", content_text(&payload["content"])),
            "function_call" => (
                "tool_call",
                tool_call_text(&payload["name"], &payload["arguments"]),
            ),
            "custom_tool_call" => (
                "tool_call",
                tool_call_text(&payload["name"], &payload["input"]),
            ),
            "tool_search_call" => (
                "tool_call",
                tool_call_text(&payload["type"], &payload["arguments"]),
            ),
            "web_search_call" | "local_shell_call" => (
                "tool_call",
                tool_call_text(&payload["type"], &payload["action"]),
            ),
            "image_generation_call" => ("tool_call", content_text(&payload["revised_prompt"])),
            "tool_search_output" => ("tool_output", argument_text(&payload["tools"])),
            "function_call_output" | "custom_tool_call_output" => {
                ("tool_output", tool_output_text(&payload["output"], query))
            }
            _ => return Vec::new(),
        };
        return vec![entry];
    }
    if row["type"] == "event_msg" {
        return match payload["type"].as_str() {
            Some("user_message") => vec![("user", content_text(&payload["message"]))],
            Some("agent_message") => vec![("assistant", content_text(&payload["message"]))],
            Some("exec_command_end") => {
                let mut parts = Vec::new();
                if let Some(code) = crate::event_msg_exit_code(payload).filter(|code| *code != 0) {
                    parts.push(format!("Exit code: {code}"));
                }
                for key in ["aggregated_output", "formatted_output", "stderr", "stdout"] {
                    if let Some(text) = payload[key].as_str().filter(|text| !text.is_empty()) {
                        parts.push(text.to_string());
                    }
                }
                vec![("tool_output", parts.join("\n"))]
            }
            _ => Vec::new(),
        };
    }
    let role = match row["type"].as_str() {
        Some("user") => "user",
        Some("assistant") => "assistant",
        Some("tool_use") => {
            return vec![(
                "tool_call",
                tool_call_text(&row["tool_name"], &row["tool_input"]),
            )]
        }
        Some("tool_result") => {
            return vec![("tool_output", tool_output_text(&row["tool_output"], query))]
        }
        _ => return Vec::new(),
    };
    let mut result = vec![(role, crate::claude_message_text(row))];
    if let Some(parts) = row["message"]["content"]
        .as_array()
        .or_else(|| row["content"].as_array())
    {
        for part in parts {
            match part["type"].as_str() {
                Some("tool_use") => {
                    result.push(("tool_call", tool_call_text(&part["name"], &part["input"])))
                }
                Some("tool_result") => result.push(("tool_output", content_text(&part["content"]))),
                _ => {}
            }
        }
    }
    result
}

fn tool_output_text(output: &Value, query: &str) -> String {
    let raw = if output.is_object() {
        output.to_string()
    } else {
        content_text(output)
    };
    // JSON notifications can use the same keys as tool envelopes. Search the
    // complete original text before decoding an escaped literal inside one.
    if query.is_empty() || raw.contains(query) {
        return raw;
    }
    let json_body = if raw.starts_with("Wall time: ") {
        raw.split_once("\nOutput:\n")
            .map_or(raw.as_str(), |(_, body)| body)
    } else {
        raw.as_str()
    };
    let decoded = serde_json::from_str::<Value>(json_body).ok();
    let value = decoded.as_ref().unwrap_or(output);
    // Index helpers normalize whitespace; literal reads need the verbatim text.
    for path in [
        "/output",
        "/preview",
        "/error",
        "/stderr",
        "/metadata/stderr",
    ] {
        if let Some(text) = value
            .pointer(path)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty() && text.contains(query))
        {
            return text.to_string();
        }
    }
    let content = if value.is_array() {
        content_text(value)
    } else {
        content_text(&value["content"])
    };
    if content.contains(query) {
        return content;
    }
    if let Some(value) = decoded {
        let text = argument_text(&value);
        if text.contains(query) {
            return text;
        }
    }
    raw
}

fn tool_call_text(name: &Value, input: &Value) -> String {
    let decoded = input
        .as_str()
        .and_then(|text| serde_json::from_str::<Value>(text).ok());
    format!(
        "{}\n{}",
        name.as_str().unwrap_or_default(),
        argument_text(decoded.as_ref().unwrap_or(input))
    )
}

fn argument_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .map(argument_text)
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(fields) => fields
            .iter()
            .map(|(key, value)| format!("{key}: {}", argument_text(value)))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => value.to_string(),
    }
}

fn add_passages(
    page: &mut Page<'_>,
    line: usize,
    kind: &str,
    text: &str,
    query: &str,
) -> AppResult<bool> {
    if text.is_empty() {
        return Ok(true);
    }
    let mut previous_end = 0;
    let matches: Box<dyn Iterator<Item = usize> + '_> = if query.is_empty() {
        Box::new(std::iter::once(0))
    } else {
        Box::new(text.match_indices(query).map(|(start, _)| start))
    };
    for position in matches {
        if position < previous_end && position + query.len() <= previous_end {
            continue;
        }
        let context_bytes = 480_usize.saturating_sub(query.len()).min(96);
        let mut start = position.saturating_sub(context_bytes);
        while !text.is_char_boundary(start) {
            start += 1;
        }
        let end = previous_char_boundary(text, (start + 480).min(text.len()));
        previous_end = end;
        if !page.push(json!({
            "line": line, "kind": kind,
            "text": &text[start..end], "text_start": start, "text_end": end,
            "text_bytes": text.len(), "truncated": start > 0 || end < text.len(),
        }))? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn read_docs(options: &Options, page: &mut Page<'_>) -> AppResult<()> {
    let input: Value = serde_json::from_reader(BufReader::new(File::open(&options.path)?))?;
    if input["isError"] == true {
        return Err("documentation search returned an MCP error".into());
    }
    let data = docs_data(input)?;
    let hits = data["hits"]
        .as_array()
        .ok_or("documentation search must contain a hits array")?;
    page.search = json!({
        "page": data["page"].as_u64(),
        "total_hits": data["nbHits"].as_u64(),
        "next_cursor": data["nextCursor"].as_str().or_else(|| data["next_cursor"].as_str()),
    });
    for (index, hit) in hits.iter().enumerate() {
        let url = hit["url"]
            .as_str()
            .filter(|url| !url.is_empty())
            .ok_or_else(|| format!("documentation hit {index} has no URL"))?;
        let title = hit["title"].as_str().map(str::to_owned).unwrap_or_else(|| {
            hit["hierarchy"]
                .as_object()
                .map(|hierarchy| {
                    hierarchy
                        .values()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" > ")
                })
                .unwrap_or_default()
        });
        let content = hit["_snippetResult"]["content"]["value"]
            .as_str()
            .or_else(|| hit["content"].as_str())
            .unwrap_or_default();
        let end = previous_char_boundary(content, content.len().min(240));
        if !page.push(json!({
            "hit": index, "url": url, "title": trim(&title, 160),
            "excerpt": &content[..end], "truncated": end < content.len(),
        }))? {
            break;
        }
    }
    Ok(())
}

fn docs_data(input: Value) -> AppResult<Value> {
    if input.get("hits").is_some() {
        return Ok(input);
    }
    if input["structuredContent"].get("hits").is_some() {
        return Ok(input["structuredContent"].clone());
    }
    if let Some(parts) = input["content"].as_array() {
        for part in parts {
            if part["type"] != "text" {
                continue;
            }
            if let Some(text) = part["text"].as_str() {
                if let Ok(data) = serde_json::from_str::<Value>(text) {
                    if data.get("hits").is_some() {
                        return Ok(data);
                    }
                }
            }
        }
    }
    Err("expected search JSON with hits, or its MCP content/structuredContent envelope".into())
}
