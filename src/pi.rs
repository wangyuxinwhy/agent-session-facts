use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::BufRead as _;
use std::io::BufReader;
use std::path::Path;
use std::path::PathBuf;

use serde_json::Map;
use serde_json::Value;
use sha2::Digest as _;
use sha2::Sha256;

use crate::error::AppError;
use crate::model::AvailabilityStatus;
use crate::model::ItemFacts;
use crate::model::LoopFacts;
use crate::model::ModelCostFacts;
use crate::model::Report;
use crate::model::RuntimeSpecificFacts;
use crate::model::SessionFacts;
use crate::model::SourceCoverageFacts;
use crate::model::SourceFacts;
use crate::model::ToolCallFacts;
use crate::model::UniformFacts;
use crate::model::Usage;
use crate::model::UsageFacts;

const MAX_SOURCE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_RECORD_BYTES: usize = 16 * 1024 * 1024;

pub(crate) fn inspect(paths: &[PathBuf]) -> Result<Report, AppError> {
    if paths.len() != 1 {
        return Err(AppError::invalid(
            "Pi stores one continuing native Session in one JSONL file; supply exactly one file",
        ));
    }
    let canonical = std::fs::canonicalize(&paths[0]).map_err(|error| {
        AppError::io(format!(
            "cannot resolve Pi session {}: {error}",
            paths[0].display()
        ))
    })?;
    let parsed = parse_source(&canonical)?;

    let mut incomplete = parsed.incomplete_reasons;
    if parsed.loops.open > 0 {
        incomplete.insert("open_loops".to_string());
    }
    if parsed.missing_usage_records > 0 {
        incomplete.insert("usage_not_reported_by_every_billed_record".to_string());
    }
    let usage = availability(
        parsed.usage,
        parsed.usage_records,
        parsed.missing_usage_records,
    );
    let model_cost = cost_facts(
        parsed.cost_usd,
        parsed.cost_records,
        parsed.usage_records,
        parsed.missing_usage_records,
    );
    let incomplete_reasons: Vec<_> = incomplete.into_iter().collect();

    Ok(Report {
        schema_version: 1,
        kind: "agent_session_facts.report",
        runtime: "pi",
        complete: incomplete_reasons.is_empty(),
        incomplete_reasons,
        uniform: UniformFacts {
            schema: "agent_session_facts.uniform@1",
            session: SessionFacts {
                native_id: parsed.session_id,
                source_count: 1,
                runtime_versions: Vec::new(),
                source_coverage: SourceCoverageFacts {
                    status: "complete",
                    basis: "single_native_pi_session_source",
                    supplied_sources: 1,
                    runtime_declared_child_sessions: 0,
                    missing_runtime_declared_child_sessions: Vec::new(),
                },
                loops: parsed.loops,
                usage,
                items: ItemFacts {
                    agent_tool_calls: parsed.tool_calls,
                },
                model_cost,
            },
            sources: vec![parsed.source],
        },
        runtime_specific: RuntimeSpecificFacts {
            schema: "agent_session_facts.pi@1",
            facts: serde_json::json!({
                "session_format_version": parsed.session_format_version,
                "created_at": parsed.created_at,
                "working_directory": parsed.working_directory,
                "parent_session": parsed.parent_session,
                "entries": {
                    "total": parsed.entry_count,
                    "by_type": parsed.entries_by_type,
                    "root_entries": parsed.root_entries,
                    "missing_parent_entry_ids": parsed.missing_parent_entry_ids,
                },
                "messages": {
                    "by_role": parsed.messages_by_role,
                    "assistant_stop_reasons": parsed.stop_reasons,
                },
                "tool_calls_by_kind": {"toolCall": parsed.tool_call_kind_count},
                "usage_accounting": {
                    "basis": "sum_of_runtime_recorded_per_response_usage",
                    "usage_records": parsed.usage_records,
                    "missing_usage_records": parsed.missing_usage_records,
                    "cost_records": parsed.cost_records,
                    "cache_write_1h_tokens": parsed.cache_write_1h,
                    "reasoning_records": parsed.reasoning_records,
                    "reasoning_output_tokens_known": parsed.reasoning_known,
                }
            }),
        },
    })
}

struct ParsedPi {
    session_id: String,
    session_format_version: Option<u64>,
    created_at: Option<String>,
    working_directory: Option<String>,
    parent_session: Option<String>,
    source: SourceFacts,
    entry_count: u64,
    entries_by_type: BTreeMap<String, u64>,
    root_entries: u64,
    missing_parent_entry_ids: Vec<String>,
    messages_by_role: BTreeMap<String, u64>,
    stop_reasons: BTreeMap<String, u64>,
    loops: LoopFacts,
    usage: Option<Usage>,
    usage_records: u64,
    missing_usage_records: u64,
    cost_usd: f64,
    cost_records: u64,
    cache_write_1h: u64,
    reasoning_records: u64,
    reasoning_known: u64,
    tool_calls: ToolCallFacts,
    tool_call_kind_count: u64,
    incomplete_reasons: BTreeSet<String>,
}

struct Header {
    id: String,
    version: Option<u64>,
    timestamp: Option<String>,
    cwd: Option<String>,
    parent_session: Option<String>,
}

#[allow(
    clippy::too_many_lines,
    reason = "the streaming fold keeps Pi entry-tree validation and accounting in source order"
)]
fn parse_source(path: &Path) -> Result<ParsedPi, AppError> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        AppError::io(format!(
            "cannot inspect Pi session {}: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(AppError::invalid(format!(
            "Pi session is not a regular file: {}",
            path.display()
        )));
    }
    if metadata.len() > MAX_SOURCE_BYTES {
        return Err(AppError::invalid(format!(
            "Pi session exceeds the {MAX_SOURCE_BYTES}-byte source limit: {}",
            path.display()
        )));
    }

    let file = File::open(path).map_err(|error| {
        AppError::io(format!(
            "cannot open Pi session {}: {error}",
            path.display()
        ))
    })?;
    let mut reader = BufReader::new(file);
    let mut buffer = Vec::new();
    let mut hasher = Sha256::new();
    let mut source_bytes = 0_u64;
    let mut records = 0_u64;
    let mut terminal_newline = false;
    let mut header = None;
    let mut entry_count = 0_u64;
    let mut entries_by_type = BTreeMap::new();
    let mut entry_ids = BTreeSet::new();
    let mut parent_ids = BTreeSet::new();
    let mut root_entries = 0_u64;
    let mut messages_by_role = BTreeMap::new();
    let mut stop_reasons = BTreeMap::new();
    let mut loops = LoopFacts::default();
    let mut active_loop = false;
    let mut usage = None;
    let mut usage_records = 0_u64;
    let mut missing_usage_records = 0_u64;
    let mut cost_usd = 0_f64;
    let mut cost_records = 0_u64;
    let mut cache_write_1h = 0_u64;
    let mut reasoning_records = 0_u64;
    let mut reasoning_known = 0_u64;
    let mut tool_calls = ToolCallFacts::default();
    let mut incomplete = BTreeSet::new();

    loop {
        buffer.clear();
        let bytes = reader.read_until(b'\n', &mut buffer).map_err(|error| {
            AppError::io(format!(
                "cannot read Pi session {}: {error}",
                path.display()
            ))
        })?;
        if bytes == 0 {
            break;
        }
        if bytes > MAX_RECORD_BYTES {
            return Err(AppError::invalid(format!(
                "Pi record {} in {} exceeds the {MAX_RECORD_BYTES}-byte record limit",
                records + 1,
                path.display()
            )));
        }
        source_bytes = add(source_bytes, usize_u64(bytes)?, "source byte count")?;
        hasher.update(&buffer);
        records = add(records, 1, "record count")?;
        terminal_newline = buffer.last() == Some(&b'\n');
        if terminal_newline {
            buffer.pop();
            if buffer.last() == Some(&b'\r') {
                buffer.pop();
            }
        }
        if buffer.is_empty() {
            return Err(AppError::invalid(format!(
                "empty JSONL record {records} in {}",
                path.display()
            )));
        }
        let value: Value = serde_json::from_slice(&buffer).map_err(|error| {
            AppError::invalid(format!(
                "invalid JSON in Pi session {} at record {records}: {error}",
                path.display()
            ))
        })?;
        let object = value.as_object().ok_or_else(|| {
            AppError::invalid(format!(
                "Pi session {} record {records} is not a JSON object",
                path.display()
            ))
        })?;
        if records == 1 {
            header = Some(parse_header(object, path)?);
            continue;
        }
        let kind = required_string(object, "type", path, records)?;
        entry_count = add(entry_count, 1, "entry count")?;
        increment(&mut entries_by_type, &kind)?;
        let id = required_string(object, "id", path, records)?;
        if !entry_ids.insert(id.clone()) {
            return Err(AppError::invalid(format!(
                "duplicate Pi entry id {id:?} in {}",
                path.display()
            )));
        }
        match object.get("parentId") {
            Some(Value::Null) => root_entries = add(root_entries, 1, "root entry count")?,
            Some(Value::String(parent)) => {
                parent_ids.insert(parent.clone());
            }
            _ => {
                return Err(AppError::invalid(format!(
                    "field \"parentId\" in {} record {records} is missing or invalid",
                    path.display()
                )));
            }
        }

        match kind.as_str() {
            "message" => process_message(
                object,
                path,
                records,
                &mut loops,
                &mut active_loop,
                &mut messages_by_role,
                &mut stop_reasons,
                &mut usage,
                &mut usage_records,
                &mut missing_usage_records,
                &mut cost_usd,
                &mut cost_records,
                &mut cache_write_1h,
                &mut reasoning_records,
                &mut reasoning_known,
                &mut tool_calls,
            )?,
            "compaction" | "branch_summary" => {
                if let Some(value) = object.get("usage") {
                    fold_usage(
                        value,
                        path,
                        records,
                        &mut usage,
                        &mut usage_records,
                        &mut cost_usd,
                        &mut cost_records,
                        &mut cache_write_1h,
                        &mut reasoning_records,
                        &mut reasoning_known,
                    )?;
                }
            }
            _ => {}
        }
    }

    let header = header
        .ok_or_else(|| AppError::invalid(format!("Pi session is empty: {}", path.display())))?;
    if active_loop {
        loops.open = add(loops.open, 1, "open loop count")?;
    }
    if !terminal_newline {
        incomplete.insert(format!(
            "source_missing_terminal_newline:{}",
            path.display()
        ));
    }
    let final_len = reader
        .get_ref()
        .metadata()
        .map_err(|error| AppError::io(format!("cannot recheck {}: {error}", path.display())))?
        .len();
    if source_bytes != metadata.len() || source_bytes != final_len {
        incomplete.insert(format!("source_changed_during_read:{}", path.display()));
    }
    let missing_parent_entry_ids = parent_ids
        .difference(&entry_ids)
        .cloned()
        .collect::<Vec<_>>();
    if !missing_parent_entry_ids.is_empty() {
        incomplete.insert("entry_tree_has_missing_parents".to_string());
    }
    let tool_call_kind_count = tool_calls.total;

    Ok(ParsedPi {
        session_id: header.id,
        session_format_version: header.version,
        created_at: header.timestamp,
        working_directory: header.cwd,
        parent_session: header.parent_session,
        source: SourceFacts {
            path: path.display().to_string(),
            sha256: sha256_string(hasher.finalize().as_slice()),
            bytes: source_bytes,
            records,
            terminal_newline,
        },
        entry_count,
        entries_by_type,
        root_entries,
        missing_parent_entry_ids,
        messages_by_role,
        stop_reasons,
        loops,
        usage,
        usage_records,
        missing_usage_records,
        cost_usd,
        cost_records,
        cache_write_1h,
        reasoning_records,
        reasoning_known,
        tool_calls,
        tool_call_kind_count,
        incomplete_reasons: incomplete,
    })
}

fn parse_header(object: &Map<String, Value>, path: &Path) -> Result<Header, AppError> {
    if object.get("type").and_then(Value::as_str) != Some("session") {
        return Err(AppError::invalid(format!(
            "{} is not a Pi session JSONL: the first record must have type \"session\"",
            path.display()
        )));
    }
    Ok(Header {
        id: required_string(object, "id", path, 1)?,
        version: optional_u64(object, "version", path, 1)?,
        timestamp: optional_string(object, "timestamp", path, 1)?,
        cwd: optional_string(object, "cwd", path, 1)?,
        parent_session: optional_string(object, "parentSession", path, 1)?,
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "the arguments are the explicit outputs of one Pi message fold"
)]
fn process_message(
    object: &Map<String, Value>,
    path: &Path,
    record: u64,
    loops: &mut LoopFacts,
    active_loop: &mut bool,
    messages_by_role: &mut BTreeMap<String, u64>,
    stop_reasons: &mut BTreeMap<String, u64>,
    usage: &mut Option<Usage>,
    usage_records: &mut u64,
    missing_usage_records: &mut u64,
    cost_usd: &mut f64,
    cost_records: &mut u64,
    cache_write_1h: &mut u64,
    reasoning_records: &mut u64,
    reasoning_known: &mut u64,
    tool_calls: &mut ToolCallFacts,
) -> Result<(), AppError> {
    let message = object
        .get("message")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            AppError::invalid(format!(
                "invalid Pi message in {} record {record}",
                path.display()
            ))
        })?;
    let role = required_string(message, "role", path, record)?;
    increment(messages_by_role, &role)?;
    if role == "user" && !*active_loop {
        loops.total = add(loops.total, 1, "loop count")?;
        *active_loop = true;
    }
    if role == "assistant" {
        if !*active_loop {
            loops.total = add(loops.total, 1, "loop count")?;
            *active_loop = true;
        }
        if let Some(value) = message.get("usage") {
            fold_usage(
                value,
                path,
                record,
                usage,
                usage_records,
                cost_usd,
                cost_records,
                cache_write_1h,
                reasoning_records,
                reasoning_known,
            )?;
        } else {
            *missing_usage_records = add(*missing_usage_records, 1, "missing usage count")?;
        }
        if let Some(content) = message.get("content").and_then(Value::as_array) {
            for block in content {
                if block.get("type").and_then(Value::as_str) == Some("toolCall") {
                    let name = block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    tool_calls.total = add(tool_calls.total, 1, "tool call count")?;
                    increment(&mut tool_calls.by_name, name)?;
                }
            }
        }
        if let Some(reason) = message.get("stopReason").and_then(Value::as_str) {
            increment(stop_reasons, reason)?;
            match reason {
                "stop" => {
                    loops.completed = add(loops.completed, 1, "completed loop count")?;
                    *active_loop = false;
                }
                "aborted" => {
                    loops.interrupted = add(loops.interrupted, 1, "interrupted loop count")?;
                    *active_loop = false;
                }
                "error" | "length" => {
                    loops.failed = add(loops.failed, 1, "failed loop count")?;
                    *active_loop = false;
                }
                _ => {}
            }
        }
    } else if role == "toolResult"
        && let Some(value) = message.get("usage")
    {
        fold_usage(
            value,
            path,
            record,
            usage,
            usage_records,
            cost_usd,
            cost_records,
            cache_write_1h,
            reasoning_records,
            reasoning_known,
        )?;
    }
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "usage and cost are separate public facts from the same native usage object"
)]
fn fold_usage(
    value: &Value,
    path: &Path,
    record: u64,
    aggregate: &mut Option<Usage>,
    usage_records: &mut u64,
    cost_usd: &mut f64,
    cost_records: &mut u64,
    cache_write_1h: &mut u64,
    reasoning_records: &mut u64,
    reasoning_known: &mut u64,
) -> Result<(), AppError> {
    let object = value.as_object().ok_or_else(|| {
        AppError::invalid(format!(
            "Pi usage in {} record {record} is not an object",
            path.display()
        ))
    })?;
    let raw_input = required_u64(object, "input", path, record)?;
    let cached = required_u64(object, "cacheRead", path, record)?;
    let cache_write = required_u64(object, "cacheWrite", path, record)?;
    let output = required_u64(object, "output", path, record)?;
    let input = add(
        add(raw_input, cached, "normalized input tokens")?,
        cache_write,
        "normalized input tokens",
    )?;
    if let Some(total) = optional_u64(object, "totalTokens", path, record)?
        && add(input, output, "total tokens")? != total
    {
        return Err(AppError::invalid(format!(
            "Pi totalTokens does not equal input + cacheRead + cacheWrite + output in {} record {record}",
            path.display()
        )));
    }
    let reasoning = optional_u64(object, "reasoning", path, record)?;
    if let Some(reasoning) = reasoning {
        *reasoning_records = add(*reasoning_records, 1, "reasoning record count")?;
        *reasoning_known = add(*reasoning_known, reasoning, "known reasoning output tokens")?;
    }
    let sample = Usage {
        input,
        cached: Some(cached),
        cache_write: Some(cache_write),
        output,
        reasoning,
    };
    merge_usage(aggregate, sample)?;
    *usage_records = add(*usage_records, 1, "usage record count")?;
    *cache_write_1h = add(
        *cache_write_1h,
        optional_u64(object, "cacheWrite1h", path, record)?.unwrap_or(0),
        "one-hour cache-write tokens",
    )?;
    if let Some(cost) = object
        .get("cost")
        .and_then(Value::as_object)
        .and_then(|cost| cost.get("total"))
        .and_then(Value::as_f64)
    {
        if !cost.is_finite() || cost < 0.0 {
            return Err(AppError::invalid(format!(
                "Pi cost.total in {} record {record} is not a finite non-negative number",
                path.display()
            )));
        }
        *cost_usd += cost;
        if !cost_usd.is_finite() {
            return Err(AppError::internal("Pi cost total overflowed"));
        }
        *cost_records = add(*cost_records, 1, "cost record count")?;
    }
    Ok(())
}

fn availability(usage: Option<Usage>, reported: u64, missing: u64) -> UsageFacts {
    match (usage, reported, missing) {
        (None, _, _) => UsageFacts {
            status: AvailabilityStatus::Unavailable,
            values: None,
            known_values: None,
        },
        (Some(values), _, 0) => UsageFacts {
            status: AvailabilityStatus::Available,
            values: Some(values),
            known_values: None,
        },
        (Some(known_values), _, _) => UsageFacts {
            status: AvailabilityStatus::Partial,
            values: None,
            known_values: Some(known_values),
        },
    }
}

fn cost_facts(
    total: f64,
    cost_records: u64,
    usage_records: u64,
    missing_usage_records: u64,
) -> ModelCostFacts {
    if usage_records == 0 || cost_records == 0 {
        ModelCostFacts {
            status: AvailabilityStatus::Unavailable,
            amount_usd: None,
            basis: None,
            reason: Some("not_recorded_in_pi_session"),
        }
    } else if cost_records == usage_records && missing_usage_records == 0 {
        ModelCostFacts {
            status: AvailabilityStatus::Available,
            amount_usd: Some(total),
            basis: Some("runtime_recorded"),
            reason: None,
        }
    } else {
        ModelCostFacts {
            status: AvailabilityStatus::Partial,
            amount_usd: Some(total),
            basis: Some("runtime_recorded_known_values"),
            reason: Some("cost_not_recorded_by_every_usage_record"),
        }
    }
}

fn merge_usage(total: &mut Option<Usage>, value: Usage) -> Result<(), AppError> {
    let Some(current) = total else {
        *total = Some(value);
        return Ok(());
    };
    current.input = add(current.input, value.input, "input tokens")?;
    current.cached = merge_optional(current.cached, value.cached, "cached input tokens")?;
    current.cache_write = merge_optional(
        current.cache_write,
        value.cache_write,
        "cache-write input tokens",
    )?;
    current.output = add(current.output, value.output, "output tokens")?;
    current.reasoning = merge_optional(
        current.reasoning,
        value.reasoning,
        "reasoning output tokens",
    )?;
    Ok(())
}

fn merge_optional(
    left: Option<u64>,
    right: Option<u64>,
    field: &str,
) -> Result<Option<u64>, AppError> {
    match (left, right) {
        (Some(left), Some(right)) => add(left, right, field).map(Some),
        _ => Ok(None),
    }
}

fn increment(values: &mut BTreeMap<String, u64>, key: &str) -> Result<(), AppError> {
    let current = values.entry(key.to_string()).or_default();
    *current = add(*current, 1, "categorized count")?;
    Ok(())
}

fn required_string(
    object: &Map<String, Value>,
    field: &str,
    path: &Path,
    record: u64,
) -> Result<String, AppError> {
    optional_string(object, field, path, record)?.ok_or_else(|| {
        AppError::invalid(format!(
            "field {field:?} in {} record {record} is missing or not a string",
            path.display()
        ))
    })
}

fn optional_string(
    object: &Map<String, Value>,
    field: &str,
    path: &Path,
    record: u64,
) -> Result<Option<String>, AppError> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(AppError::invalid(format!(
            "field {field:?} in {} record {record} is not a string",
            path.display()
        ))),
    }
}

fn required_u64(
    object: &Map<String, Value>,
    field: &str,
    path: &Path,
    record: u64,
) -> Result<u64, AppError> {
    optional_u64(object, field, path, record)?.ok_or_else(|| {
        AppError::invalid(format!(
            "field {field:?} in {} record {record} is missing or not a non-negative integer",
            path.display()
        ))
    })
}

fn optional_u64(
    object: &Map<String, Value>,
    field: &str,
    path: &Path,
    record: u64,
) -> Result<Option<u64>, AppError> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or_else(|| {
            AppError::invalid(format!(
                "field {field:?} in {} record {record} is not a non-negative integer",
                path.display()
            ))
        }),
    }
}

fn add(left: u64, right: u64, field: &str) -> Result<u64, AppError> {
    left.checked_add(right)
        .ok_or_else(|| AppError::internal(format!("{field} overflowed u64")))
}

fn usize_u64(value: usize) -> Result<u64, AppError> {
    u64::try_from(value).map_err(|_| AppError::internal("byte count does not fit in u64"))
}

fn sha256_string(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(char::from(HEX[usize::from(byte >> 4)]));
        result.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    result
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::Value;
    use serde_json::json;
    use tempfile::TempDir;

    use super::inspect;

    #[test]
    fn aggregates_pi_loops_usage_cost_and_tool_calls() {
        let temp = TempDir::new().expect("temporary directory should be created");
        let path = temp.path().join("session.jsonl");
        let records = [
            json!({"type":"session","version":3,"id":"pi-session","timestamp":"2026-08-30T00:00:00Z","cwd":"/work"}),
            json!({"type":"message","id":"u1","parentId":null,"timestamp":"2026-08-30T00:00:01Z","message":{"role":"user","content":"do it"}}),
            json!({"type":"message","id":"a1","parentId":"u1","timestamp":"2026-08-30T00:00:02Z","message":{"role":"assistant","content":[{"type":"toolCall","id":"call-1","name":"bash"}],"stopReason":"toolUse","usage":{"input":2,"output":3,"cacheRead":5,"cacheWrite":7,"totalTokens":17,"reasoning":1,"cacheWrite1h":4,"cost":{"total":0.25}}}}),
            json!({"type":"message","id":"r1","parentId":"a1","timestamp":"2026-08-30T00:00:03Z","message":{"role":"toolResult","toolCallId":"call-1","content":"ok"}}),
            json!({"type":"message","id":"a2","parentId":"r1","timestamp":"2026-08-30T00:00:04Z","message":{"role":"assistant","content":[{"type":"text","text":"done"}],"stopReason":"stop","usage":{"input":11,"output":13,"cacheRead":17,"cacheWrite":19,"totalTokens":60,"reasoning":2,"cost":{"total":0.5}}}}),
        ];
        let contents = records
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        fs::write(&path, contents).expect("fixture should be written");

        let report = inspect(&[path]).expect("valid Pi session should be inspected");
        assert!(report.complete);
        assert_eq!(report.uniform.session.loops.total, 1);
        assert_eq!(report.uniform.session.loops.completed, 1);
        let usage = report
            .uniform
            .session
            .usage
            .values
            .expect("usage should be complete");
        assert_eq!(usage.input, 61);
        assert_eq!(usage.cached, Some(22));
        assert_eq!(usage.cache_write, Some(26));
        assert_eq!(usage.output, 16);
        assert_eq!(usage.reasoning, Some(3));
        assert_eq!(report.uniform.session.items.agent_tool_calls.total, 1);
        assert_eq!(report.uniform.session.model_cost.amount_usd, Some(0.75));
        assert_eq!(
            report.runtime_specific.facts["usage_accounting"]["cache_write_1h_tokens"],
            4
        );
    }
}
