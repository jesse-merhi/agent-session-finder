use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

fn main() {
    if let Err(err) = run() {
        let _ = writeln!(io::stderr(), "agent-skill-validate: {err}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        print_help();
        return Ok(());
    }

    let inputs = if args.is_empty() {
        vec![PathBuf::from("skills")]
    } else {
        args.into_iter().map(PathBuf::from).collect()
    };

    let mut skills = Vec::new();
    let mut errors = Vec::new();
    for input in inputs {
        match discover_skills(&input) {
            Ok(mut found) => skills.append(&mut found),
            Err(err) => errors.push(format!("{}: {err}", input.display())),
        }
    }

    skills.sort();
    skills.dedup();

    if skills.is_empty() && errors.is_empty() {
        errors.push("no skill folders found".to_string());
    }

    for skill in &skills {
        errors.extend(validate_skill(skill));
    }

    if errors.is_empty() {
        for skill in &skills {
            println!("ok {}", skill.display());
        }
        println!("validated {} skill(s)", skills.len());
        Ok(())
    } else {
        for err in errors {
            eprintln!("error: {err}");
        }
        std::process::exit(1);
    }
}

fn print_help() {
    println!(
        "Validate local Codex skill folders\n\n\
usage: agent-skill-validate [PATH ...]\n\n\
PATH may be a skill folder, a SKILL.md file, or a directory containing skill folders.\n\
With no PATH, validates ./skills."
    );
}

fn discover_skills(path: &Path) -> Result<Vec<PathBuf>, String> {
    let metadata = fs::metadata(path).map_err(|err| err.to_string())?;
    if metadata.is_file() {
        if path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md") {
            return path
                .parent()
                .map(|parent| vec![parent.to_path_buf()])
                .ok_or_else(|| "SKILL.md has no parent directory".to_string());
        }
        return Err("expected a skill folder, SKILL.md file, or skills directory".to_string());
    }

    if path.join("SKILL.md").is_file() {
        return Ok(vec![path.to_path_buf()]);
    }

    let mut found = Vec::new();
    let entries = fs::read_dir(path).map_err(|err| err.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|err| err.to_string())?;
        let child = entry.path();
        if child.is_dir() && child.join("SKILL.md").is_file() {
            found.push(child);
        }
    }
    Ok(found)
}

fn validate_skill(skill_dir: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    let skill_md = skill_dir.join("SKILL.md");
    let contents = match fs::read_to_string(&skill_md) {
        Ok(contents) => contents,
        Err(err) => return vec![format!("{}: {err}", skill_md.display())],
    };

    let (frontmatter, body) = match split_frontmatter(&contents) {
        Ok(parts) => parts,
        Err(err) => return vec![format!("{}: {err}", skill_md.display())],
    };

    let metadata = match parse_frontmatter(frontmatter) {
        Ok(metadata) => metadata,
        Err(err) => {
            errors.push(format!("{}: {err}", skill_md.display()));
            BTreeMap::new()
        }
    };

    let name = metadata.get("name").cloned().unwrap_or_default();
    if name.is_empty() {
        errors.push(format!(
            "{}: missing required frontmatter key `name`",
            skill_md.display()
        ));
    } else {
        if !is_valid_skill_name(&name) {
            errors.push(format!(
                "{}: `name` must use lowercase letters, digits, and hyphens, and be under 64 characters",
                skill_md.display()
            ));
        }
        let folder = skill_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if folder != name {
            errors.push(format!(
                "{}: folder name `{folder}` must match skill name `{name}`",
                skill_dir.display()
            ));
        }
    }

    match metadata.get("description") {
        Some(description) if !description.trim().is_empty() => {}
        _ => errors.push(format!(
            "{}: missing required frontmatter key `description`",
            skill_md.display()
        )),
    }

    for key in metadata.keys() {
        if key != "name" && key != "description" {
            errors.push(format!(
                "{}: unsupported frontmatter key `{key}`; only `name` and `description` are allowed",
                skill_md.display()
            ));
        }
    }

    if body.trim().is_empty() {
        errors.push(format!("{}: markdown body is empty", skill_md.display()));
    }
    if contents.contains("TODO") || contents.contains("[TODO") {
        errors.push(format!(
            "{}: contains leftover TODO placeholder text",
            skill_md.display()
        ));
    }

    let openai_yaml = skill_dir.join("agents/openai.yaml");
    if openai_yaml.exists() {
        errors.extend(validate_openai_yaml(&openai_yaml, &name));
    }

    errors
}

fn split_frontmatter(contents: &str) -> Result<(&str, &str), String> {
    let normalized = contents.strip_prefix('\u{feff}').unwrap_or(contents);
    let mut offset = 0usize;
    for (idx, line) in normalized.split_inclusive('\n').enumerate() {
        let line_without_newline = line.trim_end_matches(['\r', '\n']);
        if idx == 0 {
            if line_without_newline.trim() != "---" {
                return Err("SKILL.md must start with YAML frontmatter delimiter `---`".to_string());
            }
        } else if line_without_newline.trim() == "---" {
            let body_start = offset + line.len();
            return Ok((&normalized[4..offset], &normalized[body_start..]));
        }
        offset += line.len();
    }
    Err("missing closing YAML frontmatter delimiter `---`".to_string())
}

fn parse_frontmatter(frontmatter: &str) -> Result<BTreeMap<String, String>, String> {
    parse_flat_mapping(frontmatter, true)
}

fn parse_flat_mapping(text: &str, allow_blocks: bool) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut index = 0usize;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            index += 1;
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            return Err(format!("line {}: unexpected indentation", index + 1));
        }
        let (key, value) = parse_key_value(line, index + 1)?;
        if out.contains_key(key) {
            return Err(format!("line {}: duplicate key `{key}`", index + 1));
        }
        let value = value.trim();
        if allow_blocks && (value == "|" || value == ">") {
            let mut parts = Vec::new();
            index += 1;
            while index < lines.len() {
                let next = lines[index];
                if next.trim().is_empty() {
                    parts.push("");
                    index += 1;
                    continue;
                }
                if !next.starts_with(' ') && !next.starts_with('\t') {
                    break;
                }
                parts.push(next.trim());
                index += 1;
            }
            let joined = if value == ">" {
                parts.join(" ")
            } else {
                parts.join("\n")
            };
            out.insert(key.to_string(), joined.trim().to_string());
            continue;
        }
        out.insert(key.to_string(), parse_scalar(value).0);
        index += 1;
    }
    Ok(out)
}

fn parse_key_value(line: &str, line_number: usize) -> Result<(&str, &str), String> {
    let (key, value) = line
        .split_once(':')
        .ok_or_else(|| format!("line {line_number}: expected `key: value`"))?;
    let key = key.trim();
    if key.is_empty() {
        return Err(format!("line {line_number}: empty key"));
    }
    if key.contains(' ') || key.contains('\t') {
        return Err(format!(
            "line {line_number}: key `{key}` contains whitespace"
        ));
    }
    Ok((key, value))
}

fn parse_scalar(value: &str) -> (String, bool) {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        return (unescape_double_quoted(&value[1..value.len() - 1]), true);
    }
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        return (value[1..value.len() - 1].to_string(), true);
    }

    let without_comment = value
        .split_once(" #")
        .map(|(before, _)| before)
        .unwrap_or(value)
        .trim();
    (without_comment.to_string(), false)
}

fn unescape_double_quoted(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn is_valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn validate_openai_yaml(path: &Path, skill_name: &str) -> Vec<String> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) => return vec![format!("{}: {err}", path.display())],
    };
    let mut errors = Vec::new();
    let mut top_level = BTreeSet::new();
    let mut interface = BTreeMap::new();
    let mut current_top: Option<String> = None;

    for (line_number, line) in contents.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if line.starts_with('\t') {
            errors.push(format!(
                "{}:{}: tabs are not valid indentation",
                path.display(),
                line_number + 1
            ));
            continue;
        }
        let indent = line.len() - line.trim_start_matches(' ').len();
        if indent == 0 {
            match parse_key_value(line, line_number + 1) {
                Ok((key, value)) => {
                    if !matches!(key, "interface" | "dependencies" | "policy") {
                        errors.push(format!(
                            "{}:{}: unsupported top-level key `{key}`",
                            path.display(),
                            line_number + 1
                        ));
                    }
                    top_level.insert(key.to_string());
                    current_top = Some(key.to_string());
                    if !value.trim().is_empty() {
                        errors.push(format!(
                            "{}:{}: top-level key `{key}` must contain a nested mapping",
                            path.display(),
                            line_number + 1
                        ));
                    }
                }
                Err(err) => errors.push(format!("{}: {err}", path.display())),
            }
            continue;
        }

        if current_top.as_deref() == Some("interface") && indent == 2 {
            match parse_key_value(trimmed, line_number + 1) {
                Ok((key, value)) => {
                    if !matches!(
                        key,
                        "display_name"
                            | "short_description"
                            | "icon_small"
                            | "icon_large"
                            | "brand_color"
                            | "default_prompt"
                    ) {
                        errors.push(format!(
                            "{}:{}: unsupported interface key `{key}`",
                            path.display(),
                            line_number + 1
                        ));
                    }
                    let (parsed, quoted) = parse_scalar(value);
                    interface.insert(key.to_string(), (parsed, quoted));
                }
                Err(err) => errors.push(format!("{}: {err}", path.display())),
            }
        }
    }

    if !top_level.contains("interface") {
        errors.push(format!("{}: missing `interface` mapping", path.display()));
    }

    for required in ["display_name", "short_description", "default_prompt"] {
        match interface.get(required) {
            Some((value, quoted)) if !value.trim().is_empty() => {
                if !quoted {
                    errors.push(format!(
                        "{}: interface.{required} must be quoted",
                        path.display()
                    ));
                }
            }
            _ => errors.push(format!(
                "{}: missing non-empty interface.{required}",
                path.display()
            )),
        }
    }

    if let Some((short_description, _)) = interface.get("short_description") {
        let len = short_description.chars().count();
        if !(25..=64).contains(&len) {
            errors.push(format!(
                "{}: interface.short_description must be 25-64 characters",
                path.display()
            ));
        }
    }

    if !skill_name.is_empty() {
        if let Some((default_prompt, _)) = interface.get("default_prompt") {
            let needle = format!("${skill_name}");
            if !default_prompt.contains(&needle) {
                errors.push(format!(
                    "{}: interface.default_prompt must mention `{needle}`",
                    path.display()
                ));
            }
        }
    }

    errors
}
