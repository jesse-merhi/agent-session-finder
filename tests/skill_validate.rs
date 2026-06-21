use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[test]
fn validates_repo_session_recall_skill() {
    let skill = Path::new(env!("CARGO_MANIFEST_DIR")).join("skills/session-recall");
    let output = Command::new(env!("CARGO_BIN_EXE_agent-skill-validate"))
        .arg(skill)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("validated 1 skill(s)"), "{stdout}");
}

#[test]
fn validates_skills_directory_inputs() {
    let root = unique_temp_dir();
    write_skill(
        &root.join("skills/alpha-skill"),
        "alpha-skill",
        "$alpha-skill",
    );
    write_skill(&root.join("skills/beta-skill"), "beta-skill", "$beta-skill");

    let output = Command::new(env!("CARGO_BIN_EXE_agent-skill-validate"))
        .arg(root.join("skills"))
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("validated 2 skill(s)"), "{stdout}");
}

#[test]
fn rejects_invalid_frontmatter_and_folder_mismatch() {
    let root = unique_temp_dir();
    let skill = root.join("Bad Name");
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: Bad Name\ndescription: nope\nmetadata: extra\n---\n# Bad\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_agent-skill-validate"))
        .arg(&skill)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("lowercase letters"), "{stderr}");
    assert!(
        stderr.contains("unsupported frontmatter key `metadata`"),
        "{stderr}"
    );
}

#[test]
fn rejects_stale_openai_default_prompt() {
    let root = unique_temp_dir();
    let skill = root.join("session-recall");
    write_skill(&skill, "session-recall", "$other-skill");

    let output = Command::new(env!("CARGO_BIN_EXE_agent-skill-validate"))
        .arg(&skill)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("interface.default_prompt must mention `$session-recall`"),
        "{stderr}"
    );
}

#[test]
fn rejects_unquoted_openai_interface_strings() {
    let root = unique_temp_dir();
    let skill = root.join("quote-check");
    fs::create_dir_all(skill.join("agents")).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: quote-check\ndescription: Check quote validation.\n---\n# Quote Check\n",
    )
    .unwrap();
    fs::write(
        skill.join("agents/openai.yaml"),
        "interface:\n  display_name: Quote Check\n  short_description: Find invalid quote metadata\n  default_prompt: Use $quote-check now.\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_agent-skill-validate"))
        .arg(&skill)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("interface.display_name must be quoted"),
        "{stderr}"
    );
    assert!(
        stderr.contains("interface.short_description must be quoted"),
        "{stderr}"
    );
    assert!(
        stderr.contains("interface.default_prompt must be quoted"),
        "{stderr}"
    );
}

fn write_skill(skill: &Path, name: &str, prompt_skill: &str) {
    fs::create_dir_all(skill.join("agents")).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        format!(
            "---\nname: {name}\ndescription: Find local sessions for testing.\n---\n# {}\n\nUse the skill.\n",
            name.replace('-', " ")
        ),
    )
    .unwrap();
    fs::write(
        skill.join("agents/openai.yaml"),
        format!(
            "interface:\n  display_name: \"{}\"\n  short_description: \"Find local sessions for testing\"\n  default_prompt: \"Use {prompt_skill} for this.\"\n",
            name.replace('-', " ")
        ),
    )
    .unwrap();
}

fn unique_temp_dir() -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "agent-skill-validate-test-{}-{nanos}-{counter}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}
