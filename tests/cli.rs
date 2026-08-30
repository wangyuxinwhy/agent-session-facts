use std::collections::BTreeSet;
use std::fs;
use std::process::Command;

use serde_json::Value;
use serde_json::json;
use tempfile::TempDir;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_agent-session-facts"))
}

#[test]
fn help_documents_the_single_inspection_workflow() {
    let output = binary().arg("--help").output().expect("CLI should start");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("help should be UTF-8");
    assert!(stdout.contains("loop outcomes"));
    assert!(stdout.contains("cache-read, cache-write, and reasoning breakdowns"));
    assert!(stdout.contains("tool calls"));
    assert!(stdout.contains("source coverage"));
    assert!(stdout.contains("Usage: agent-session-facts --runtime <RUNTIME> <ARTIFACT>..."));
    assert!(stdout.contains("Codex   1 to 1024 on-disk rollout JSONL files"));
    assert!(stdout.contains("Pi      Exactly one native Session JSONL file"));
    assert!(stdout.contains("Claude  1 to 1024 JSONL files"));
    assert!(stdout.contains("agent_session_facts.report"));
    assert!(stdout.contains("agent_session_facts.error"));
    assert!(stdout.contains("cached       Input tokens read from an existing prompt cache"));
    assert!(stdout.contains("cache_write  Input tokens newly stored for later requests"));
    assert!(stdout.contains("Do not add cached or cache_write to input"));
    assert!(stdout.contains("A null field is unknown; zero is a known zero"));
    assert!(stdout.contains("complete=false"));
    assert!(stdout.contains("Invalid or unsupported input"));
    assert!(stdout.contains("codex exec --json"));
    assert!(stdout.contains("agent-session-facts --runtime codex root.jsonl child.jsonl"));
}

#[test]
fn subcommands_are_not_accepted() {
    for argument in ["help", "session", "inspect"] {
        let output = binary().arg(argument).output().expect("CLI should start");
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        let error: Value =
            serde_json::from_slice(&output.stderr).expect("stderr should be structured JSON");
        assert_eq!(error["type"], "agent_session_facts.error");
        assert_eq!(error["category"], "invalid_input");
    }
}

#[test]
fn argument_errors_are_one_structured_stderr_object() {
    let output = binary()
        .args(["--runtime", "codex"])
        .output()
        .expect("CLI should start");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.ends_with(b"\n"));
    assert!(!output.stderr[..output.stderr.len() - 1].contains(&b'\n'));
    let error: Value = serde_json::from_slice(&output.stderr).expect("stderr should be JSON");
    assert_eq!(error["type"], "agent_session_facts.error");
    assert_eq!(error["category"], "invalid_input");
    let message = error["message"].as_str().expect("message should be text");
    assert_eq!(message.matches("Usage:").count(), 1);
    assert!(message.contains("agent-session-facts --runtime"));
}

#[test]
fn missing_source_is_a_structured_io_error() {
    let output = binary()
        .args(["--runtime", "codex"])
        .arg("/path/that/does/not/exist.jsonl")
        .output()
        .expect("CLI should start");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).expect("stderr should be JSON");
    assert_eq!(error["category"], "io");
}

#[test]
fn inspect_emits_one_compact_json_report() {
    let temp = TempDir::new().expect("temporary directory should be created");
    let rollout = temp.path().join("rollout.jsonl");
    let records = [
        json!({
            "timestamp":"2026-08-30T00:00:00Z",
            "ordinal":0,
            "type":"session_meta",
            "payload":{
                "session_id":"session",
                "id":"thread",
                "timestamp":"2026-08-30T00:00:00Z",
                "cwd":"/workspace",
                "originator":"codex_cli_rs",
                "cli_version":"0.150.1",
                "history_mode":"paginated",
                "history_base":null
            }
        }),
        json!({
            "timestamp":"2026-08-30T00:00:01Z",
            "ordinal":1,
            "type":"event_msg",
            "payload":{"type":"task_started","turn_id":"turn"}
        }),
        json!({
            "timestamp":"2026-08-30T00:00:02Z",
            "ordinal":2,
            "type":"event_msg",
            "payload":{
                "type":"token_count",
                "info":{
                    "total_token_usage":{
                        "input_tokens":4,
                        "cached_input_tokens":2,
                        "cache_write_input_tokens":0,
                        "output_tokens":1,
                        "reasoning_output_tokens":0,
                        "total_tokens":5
                    },
                    "last_token_usage":{
                        "input_tokens":4,
                        "cached_input_tokens":2,
                        "cache_write_input_tokens":0,
                        "output_tokens":1,
                        "reasoning_output_tokens":0,
                        "total_tokens":5
                    },
                    "model_context_window":1000
                },
                "rate_limits":null
            }
        }),
        json!({
            "timestamp":"2026-08-30T00:00:03Z",
            "ordinal":3,
            "type":"event_msg",
            "payload":{"type":"task_complete","turn_id":"turn","last_agent_message":null}
        }),
    ];
    let contents = records
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(&rollout, contents).expect("fixture should be written");

    let output = binary()
        .args(["--runtime", "codex"])
        .arg(&rollout)
        .output()
        .expect("CLI should start");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(output.stdout.ends_with(b"\n"));
    assert!(!output.stdout[..output.stdout.len() - 1].contains(&b'\n'));
    let report: Value = serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert_uniform_contract(&report);
    assert_eq!(report["type"], "agent_session_facts.report");
    assert_eq!(report["complete"], true);
    assert_eq!(report["uniform"]["schema"], "agent_session_facts.uniform@1");
    assert_eq!(report["uniform"]["session"]["usage"]["values"]["input"], 4);
    assert_eq!(report["uniform"]["session"]["usage"]["values"]["output"], 1);
    assert_eq!(
        report["runtime_specific"]["facts"]["usage_accounting"]["counter_resets"],
        0
    );
}

#[test]
fn incomplete_source_still_emits_a_successful_report() {
    let temp = TempDir::new().expect("temporary directory should be created");
    let rollout = temp.path().join("active.jsonl");
    let records = [
        json!({
            "timestamp":"2026-08-30T00:00:00Z",
            "ordinal":0,
            "type":"session_meta",
            "payload":{
                "session_id":"session",
                "id":"thread",
                "timestamp":"2026-08-30T00:00:00Z",
                "cwd":"/workspace",
                "originator":"codex_cli_rs",
                "cli_version":"0.150.1",
                "history_mode":"paginated",
                "history_base":null
            }
        }),
        json!({
            "timestamp":"2026-08-30T00:00:01Z",
            "ordinal":1,
            "type":"event_msg",
            "payload":{"type":"task_started","turn_id":"open"}
        }),
    ];
    let contents = records
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&rollout, contents).expect("fixture should be written");

    let output = binary()
        .args(["--runtime", "codex"])
        .arg(&rollout)
        .output()
        .expect("CLI should start");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let report: Value = serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert_uniform_contract(&report);
    assert_eq!(report["complete"], false);
    assert_eq!(report["uniform"]["session"]["loops"]["open"], 1);
    assert!(
        report["incomplete_reasons"]
            .as_array()
            .is_some_and(|reasons| reasons.iter().any(|reason| reason
                .as_str()
                .is_some_and(|value| value.starts_with("source_missing_terminal_newline:"))))
    );
}

#[test]
fn exec_json_dialect_fails_with_structured_stderr() {
    let temp = TempDir::new().expect("temporary directory should be created");
    let rollout = temp.path().join("events.jsonl");
    fs::write(
        &rollout,
        "{\"type\":\"thread.started\",\"thread_id\":\"x\"}\n",
    )
    .expect("fixture should be written");

    let output = binary()
        .args(["--runtime", "codex"])
        .arg(&rollout)
        .output()
        .expect("CLI should start");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).expect("stderr should be JSON");
    assert_eq!(error["type"], "agent_session_facts.error");
    assert_eq!(error["category"], "invalid_input");
    assert!(
        error["message"]
            .as_str()
            .is_some_and(|message| message.contains("codex exec --json"))
    );
}

#[test]
fn pi_emits_the_uniform_and_runtime_specific_schemas() {
    let temp = TempDir::new().expect("temporary directory should be created");
    let session = temp.path().join("pi.jsonl");
    let records = [
        json!({"type":"session","version":3,"id":"pi-session","timestamp":"2026-08-30T00:00:00Z","cwd":"/work"}),
        json!({"type":"message","id":"u","parentId":null,"timestamp":"2026-08-30T00:00:01Z","message":{"role":"user","content":"hello"}}),
        json!({"type":"message","id":"a","parentId":"u","timestamp":"2026-08-30T00:00:02Z","message":{"role":"assistant","content":[{"type":"text","text":"done"}],"stopReason":"stop","usage":{"input":2,"cacheRead":3,"cacheWrite":5,"output":7,"totalTokens":17,"reasoning":1,"cost":{"total":0.25}}}}),
    ];
    write_jsonl(&session, &records, true);

    let output = binary()
        .args(["--runtime", "pi"])
        .arg(&session)
        .output()
        .expect("CLI should start");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let report: Value = serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert_uniform_contract(&report);
    assert_eq!(report["runtime"], "pi");
    assert_eq!(report["uniform"]["schema"], "agent_session_facts.uniform@1");
    assert_eq!(
        report["runtime_specific"]["schema"],
        "agent_session_facts.pi@1"
    );
    assert_eq!(report["uniform"]["session"]["loops"]["completed"], 1);
    assert_eq!(report["uniform"]["session"]["usage"]["values"]["input"], 10);
}

#[test]
fn claude_emits_the_uniform_and_runtime_specific_schemas() {
    let temp = TempDir::new().expect("temporary directory should be created");
    let session = temp.path().join("claude.jsonl");
    let records = [
        json!({"type":"user","uuid":"u","parentUuid":null,"sessionId":"claude-session","isSidechain":false,"promptId":"p","version":"2.1.1","message":{"role":"user","content":"hello"}}),
        json!({"type":"assistant","uuid":"a","parentUuid":"u","sessionId":"claude-session","isSidechain":false,"version":"2.1.1","message":{"id":"m","role":"assistant","stop_reason":"end_turn","usage":{"input_tokens":2,"cache_read_input_tokens":3,"cache_creation_input_tokens":5,"output_tokens":7,"output_tokens_details":{"thinking_tokens":1}},"content":[{"type":"text","text":"done"}]}}),
    ];
    write_jsonl(&session, &records, true);

    let output = binary()
        .args(["--runtime", "claude"])
        .arg(&session)
        .output()
        .expect("CLI should start");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let report: Value = serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert_uniform_contract(&report);
    assert_eq!(report["runtime"], "claude");
    assert_eq!(report["uniform"]["schema"], "agent_session_facts.uniform@1");
    assert_eq!(
        report["runtime_specific"]["schema"],
        "agent_session_facts.claude@1"
    );
    assert_eq!(report["uniform"]["session"]["loops"]["completed"], 1);
    assert_eq!(report["uniform"]["session"]["usage"]["values"]["input"], 10);
}

fn write_jsonl(path: &std::path::Path, records: &[Value], terminal_newline: bool) {
    let mut contents = records
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    if terminal_newline {
        contents.push('\n');
    }
    fs::write(path, contents).expect("fixture should be written");
}

fn assert_uniform_contract(report: &Value) {
    assert_object_keys(
        report,
        &[
            "complete",
            "incomplete_reasons",
            "runtime",
            "runtime_specific",
            "schema_version",
            "type",
            "uniform",
        ],
    );
    assert_object_keys(&report["uniform"], &["schema", "session", "sources"]);
    assert_object_keys(
        &report["uniform"]["session"],
        &[
            "items",
            "loops",
            "model_cost",
            "native_id",
            "runtime_versions",
            "source_count",
            "source_coverage",
            "usage",
        ],
    );
    assert_object_keys(
        &report["uniform"]["session"]["usage"],
        &["known_values", "status", "values"],
    );
    for field in ["values", "known_values"] {
        let usage = &report["uniform"]["session"]["usage"][field];
        if !usage.is_null() {
            assert_object_keys(
                usage,
                &["cache_write", "cached", "input", "output", "reasoning"],
            );
        }
    }
    assert_object_keys(
        &report["uniform"]["session"]["source_coverage"],
        &[
            "basis",
            "missing_runtime_declared_child_sessions",
            "runtime_declared_child_sessions",
            "status",
            "supplied_sources",
        ],
    );
    assert_object_keys(
        &report["uniform"]["session"]["loops"],
        &[
            "completed",
            "ended_without_outcome",
            "failed",
            "interrupted",
            "open",
            "total",
        ],
    );
    assert_object_keys(&report["uniform"]["session"]["items"], &["agent.tool_call"]);
    assert_object_keys(
        &report["uniform"]["session"]["items"]["agent.tool_call"],
        &["by_name", "total"],
    );
    assert_object_keys(
        &report["uniform"]["session"]["model_cost"],
        &["amount_usd", "basis", "reason", "status"],
    );
    assert_object_keys(&report["runtime_specific"], &["facts", "schema"]);
    for source in report["uniform"]["sources"]
        .as_array()
        .expect("sources should be an array")
    {
        assert_object_keys(
            source,
            &["bytes", "path", "records", "sha256", "terminal_newline"],
        );
    }
}

fn assert_object_keys(value: &Value, expected: &[&str]) {
    let actual = value
        .as_object()
        .expect("contract value should be an object")
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
}
