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
const MAX_SOURCES: usize = 1024;
const MAX_RESPONSE_IDS: usize = 1_000_000;

#[allow(
    clippy::too_many_lines,
    reason = "Session assembly keeps cross-source identity, coverage, and accounting visible"
)]
pub(crate) fn inspect(paths: &[PathBuf]) -> Result<Report, AppError> {
    if paths.is_empty() {
        return Err(AppError::invalid(
            "at least one Claude Code session JSONL file is required",
        ));
    }
    if paths.len() > MAX_SOURCES {
        return Err(AppError::invalid(format!(
            "at most {MAX_SOURCES} Claude Code sources may be inspected at once"
        )));
    }
    let mut canonical_paths = BTreeSet::new();
    let mut parsed = Vec::with_capacity(paths.len());
    for path in paths {
        let canonical = std::fs::canonicalize(path).map_err(|error| {
            AppError::io(format!(
                "cannot resolve Claude Code session {}: {error}",
                path.display()
            ))
        })?;
        if !canonical_paths.insert(canonical.clone()) {
            return Err(AppError::invalid(format!(
                "Claude Code source was supplied more than once: {}",
                canonical.display()
            )));
        }
        parsed.push(parse_source(&canonical)?);
    }

    let native_id = parsed
        .first()
        .map(|source| source.session_id.clone())
        .ok_or_else(|| AppError::internal("parsed Claude source set is empty"))?;
    for source in &parsed {
        if source.session_id != native_id {
            return Err(AppError::invalid(format!(
                "Claude Code sources belong to different native sessions: expected {native_id}, found {} in {}",
                source.session_id, source.source.path
            )));
        }
    }
    let root_sources = parsed
        .iter()
        .filter(|source| source.agent_id.is_none())
        .count();
    if root_sources != 1 {
        return Err(AppError::invalid(format!(
            "a Claude Code Session inspection requires exactly one root source; found {root_sources}"
        )));
    }

    let supplied_agents = parsed
        .iter()
        .filter_map(|source| source.agent_id.clone())
        .collect::<BTreeSet<_>>();
    if supplied_agents.len() + 1 != parsed.len() {
        return Err(AppError::invalid(
            "multiple Claude Code sources describe the same root or subagent context",
        ));
    }
    let declared_agents = parsed
        .iter()
        .flat_map(|source| source.declared_agents.iter().cloned())
        .collect::<BTreeSet<_>>();
    let missing_agents = declared_agents
        .difference(&supplied_agents)
        .cloned()
        .collect::<Vec<_>>();

    let mut incomplete = BTreeSet::new();
    let mut loops = LoopFacts::default();
    let mut runtime_versions = BTreeSet::new();
    let mut responses = BTreeMap::new();
    let mut response_usage_records = 0_u64;
    let mut progressive_usage_snapshots = 0_u64;
    let mut duplicate_usage_snapshots = 0_u64;
    let mut usage_without_message_id = 0_u64;
    let mut tools_by_id = BTreeMap::new();
    let mut tool_kinds = BTreeMap::new();
    let mut contexts = Vec::with_capacity(parsed.len());
    let mut sources = Vec::with_capacity(parsed.len());

    for source in parsed {
        incomplete.extend(source.incomplete_reasons.iter().cloned());
        add_loops(&mut loops, &source.loops)?;
        runtime_versions.extend(source.runtime_versions.iter().cloned());
        response_usage_records = add(
            response_usage_records,
            source.response_usage_records,
            "assistant usage record count",
        )?;
        progressive_usage_snapshots = add(
            progressive_usage_snapshots,
            source.progressive_usage_snapshots,
            "progressive usage snapshot count",
        )?;
        duplicate_usage_snapshots = add(
            duplicate_usage_snapshots,
            source.duplicate_usage_snapshots,
            "duplicate usage snapshot count",
        )?;
        usage_without_message_id = add(
            usage_without_message_id,
            source.usage_without_message_id,
            "usage without message id count",
        )?;
        for (id, value) in source.responses {
            if responses.insert(id.clone(), value).is_some() {
                return Err(AppError::unsupported(format!(
                    "Claude assistant message identity {id:?} appears in more than one supplied context"
                )));
            }
            if responses.len() > MAX_RESPONSE_IDS {
                return Err(AppError::invalid(format!(
                    "Claude Session exceeds the {MAX_RESPONSE_IDS}-response accounting limit"
                )));
            }
        }
        for (id, tool) in source.tools_by_id {
            match tools_by_id.get(&id) {
                Some(existing) if existing != &tool => {
                    return Err(AppError::invalid(format!(
                        "Claude tool id {id:?} has conflicting names"
                    )));
                }
                Some(_) => {}
                None => {
                    tools_by_id.insert(id, tool);
                }
            }
        }
        add_counts(&mut tool_kinds, &source.tool_kinds, "tool kind count")?;
        contexts.push(serde_json::json!({
            "agent_id": source.agent_id,
            "is_sidechain": source.is_sidechain,
            "records_by_type": source.records_by_type,
            "record_tree": {
                "identified_records": source.identified_records,
                "root_records": source.root_records,
                "missing_parent_uuids": source.missing_parent_uuids,
            },
            "loops": source.loops,
            "assistant_usage": {
                "physical_records": source.response_usage_records,
                "unique_message_ids": source.unique_response_count,
                "progressive_snapshots": source.progressive_usage_snapshots,
                "duplicate_snapshots": source.duplicate_usage_snapshots,
                "records_without_message_id": source.usage_without_message_id,
            }
        }));
        sources.push(source.source);
    }
    if !missing_agents.is_empty() {
        for agent in &missing_agents {
            incomplete.insert(format!("missing_runtime_declared_child_session:{agent}"));
        }
    }
    if loops.open > 0 {
        incomplete.insert("open_loops".to_string());
    }
    if usage_without_message_id > 0 {
        incomplete.insert("assistant_usage_without_message_id".to_string());
    }

    let unique_response_count = usize_u64(responses.len())?;
    let reasoning_response_count = usize_u64(
        responses
            .values()
            .filter(|sample| sample.reasoning.is_some())
            .count(),
    )?;
    let mut reasoning_known = 0_u64;
    for reasoning in responses.values().filter_map(|sample| sample.reasoning) {
        reasoning_known = add(reasoning_known, reasoning, "known reasoning output tokens")?;
    }
    let mut usage = None;
    for sample in responses.into_values() {
        merge_usage(&mut usage, sample)?;
    }
    let usage_facts = if usage_without_message_id == 0 {
        match usage {
            Some(values) => UsageFacts {
                status: AvailabilityStatus::Available,
                values: Some(values),
                known_values: None,
            },
            None => UsageFacts {
                status: AvailabilityStatus::Unavailable,
                values: None,
                known_values: None,
            },
        }
    } else {
        UsageFacts {
            status: AvailabilityStatus::Partial,
            values: None,
            known_values: usage,
        }
    };
    let mut tool_calls = ToolCallFacts::default();
    for name in tools_by_id.into_values() {
        tool_calls.total = add(tool_calls.total, 1, "tool call count")?;
        increment(&mut tool_calls.by_name, &name)?;
    }
    contexts.sort_by(|left, right| left["agent_id"].as_str().cmp(&right["agent_id"].as_str()));
    sources.sort_by(|left, right| left.path.cmp(&right.path));
    let source_count = usize_u64(sources.len())?;
    let declared_count = usize_u64(declared_agents.len())?;
    let incomplete_reasons = incomplete.into_iter().collect::<Vec<_>>();

    Ok(Report {
        schema_version: 1,
        kind: "agent_session_facts.report",
        runtime: "claude",
        complete: incomplete_reasons.is_empty(),
        incomplete_reasons,
        uniform: UniformFacts {
            schema: "agent_session_facts.uniform@1",
            session: SessionFacts {
                native_id,
                source_count,
                runtime_versions: runtime_versions.into_iter().collect(),
                source_coverage: SourceCoverageFacts {
                    status: if missing_agents.is_empty() {
                        "complete"
                    } else {
                        "incomplete"
                    },
                    basis: "supplied_sources_and_runtime_declared_child_sessions",
                    supplied_sources: source_count,
                    runtime_declared_child_sessions: declared_count,
                    missing_runtime_declared_child_sessions: missing_agents,
                },
                loops,
                usage: usage_facts,
                items: ItemFacts {
                    agent_tool_calls: tool_calls,
                },
                model_cost: ModelCostFacts {
                    status: AvailabilityStatus::Unavailable,
                    amount_usd: None,
                    basis: None,
                    reason: Some("not_recorded_in_claude_session"),
                },
            },
            sources,
        },
        runtime_specific: RuntimeSpecificFacts {
            schema: "agent_session_facts.claude@1",
            facts: serde_json::json!({
                "contexts": contexts,
                "tool_calls_by_kind": tool_kinds,
                "usage_accounting": {
                    "basis": "latest_monotonic_snapshot_per_assistant_message_id",
                    "physical_records": response_usage_records,
                    "unique_message_ids": unique_response_count,
                    "progressive_snapshots": progressive_usage_snapshots,
                    "duplicate_snapshots": duplicate_usage_snapshots,
                    "records_without_message_id": usage_without_message_id,
                    "reasoning_response_ids": reasoning_response_count,
                    "reasoning_output_tokens_known": reasoning_known,
                }
            }),
        },
    })
}

struct ParsedClaude {
    session_id: String,
    agent_id: Option<String>,
    is_sidechain: Option<bool>,
    source: SourceFacts,
    runtime_versions: BTreeSet<String>,
    records_by_type: BTreeMap<String, u64>,
    identified_records: u64,
    root_records: u64,
    missing_parent_uuids: Vec<String>,
    declared_agents: BTreeSet<String>,
    loops: LoopFacts,
    responses: BTreeMap<String, Usage>,
    response_usage_records: u64,
    unique_response_count: u64,
    progressive_usage_snapshots: u64,
    duplicate_usage_snapshots: u64,
    usage_without_message_id: u64,
    tools_by_id: BTreeMap<String, String>,
    tool_kinds: BTreeMap<String, u64>,
    incomplete_reasons: BTreeSet<String>,
}

#[allow(
    clippy::too_many_lines,
    reason = "the streaming fold keeps Claude record-tree, Loop, response, and tool identities together"
)]
fn parse_source(path: &Path) -> Result<ParsedClaude, AppError> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        AppError::io(format!(
            "cannot inspect Claude source {}: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(AppError::invalid(format!(
            "Claude source is not a regular file: {}",
            path.display()
        )));
    }
    if metadata.len() > MAX_SOURCE_BYTES {
        return Err(AppError::invalid(format!(
            "Claude source exceeds the {MAX_SOURCE_BYTES}-byte source limit: {}",
            path.display()
        )));
    }
    let file = File::open(path).map_err(|error| {
        AppError::io(format!(
            "cannot open Claude source {}: {error}",
            path.display()
        ))
    })?;
    let mut reader = BufReader::new(file);
    let mut buffer = Vec::new();
    let mut hasher = Sha256::new();
    let mut source_bytes = 0_u64;
    let mut records = 0_u64;
    let mut terminal_newline = false;
    let mut session_ids = BTreeSet::new();
    let mut agent_ids = BTreeSet::new();
    let mut sidechain_values = BTreeSet::new();
    let mut versions = BTreeSet::new();
    let mut records_by_type = BTreeMap::new();
    let mut uuids = BTreeSet::new();
    let mut parent_uuids = BTreeSet::new();
    let mut root_records = 0_u64;
    let mut declared_agents = BTreeSet::new();
    let mut duration_boundaries = 0_u64;
    let mut interrupted_boundaries = 0_u64;
    let mut active_loop = false;
    let mut terminal_response_ids = BTreeSet::new();
    let mut responses = BTreeMap::new();
    let mut response_usage_records = 0_u64;
    let mut progressive_usage_snapshots = 0_u64;
    let mut duplicate_usage_snapshots = 0_u64;
    let mut usage_without_message_id = 0_u64;
    let mut tools_by_id = BTreeMap::new();
    let mut tool_kinds = BTreeMap::new();
    let mut incomplete = BTreeSet::new();

    loop {
        buffer.clear();
        let bytes = reader.read_until(b'\n', &mut buffer).map_err(|error| {
            AppError::io(format!(
                "cannot read Claude source {}: {error}",
                path.display()
            ))
        })?;
        if bytes == 0 {
            break;
        }
        if bytes > MAX_RECORD_BYTES {
            return Err(AppError::invalid(format!(
                "Claude record {} in {} exceeds the {MAX_RECORD_BYTES}-byte record limit",
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
                "invalid JSON in Claude source {} at record {records}: {error}",
                path.display()
            ))
        })?;
        let object = value.as_object().ok_or_else(|| {
            AppError::invalid(format!(
                "Claude source {} record {records} is not a JSON object",
                path.display()
            ))
        })?;
        if let Some(id) = object.get("sessionId").and_then(Value::as_str) {
            session_ids.insert(id.to_string());
        }
        if let Some(id) = object.get("agentId").and_then(Value::as_str) {
            agent_ids.insert(id.to_string());
        }
        if let Some(sidechain) = object.get("isSidechain").and_then(Value::as_bool) {
            sidechain_values.insert(sidechain);
        }
        if let Some(version) = object.get("version").and_then(Value::as_str) {
            versions.insert(version.to_string());
        }
        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        increment(&mut records_by_type, kind)?;
        if let Some(uuid) = object.get("uuid").and_then(Value::as_str) {
            if !uuids.insert(uuid.to_string()) {
                return Err(AppError::invalid(format!(
                    "duplicate Claude record uuid {uuid:?} in {}",
                    path.display()
                )));
            }
            match object.get("parentUuid") {
                Some(Value::String(parent)) => {
                    parent_uuids.insert(parent.clone());
                }
                None | Some(Value::Null) => {
                    root_records = add(root_records, 1, "root record count")?;
                }
                Some(_) => {
                    return Err(AppError::invalid(format!(
                        "invalid parentUuid in {} record {records}",
                        path.display()
                    )));
                }
            }
        }
        if let Some(result) = object.get("toolUseResult") {
            collect_agent_ids(result, &mut declared_agents);
        }

        let interrupted = object
            .get("interruptedMessageId")
            .is_some_and(|value| !value.is_null());
        let duration = kind == "system"
            && object.get("subtype").and_then(Value::as_str) == Some("turn_duration");
        if interrupted {
            interrupted_boundaries = add(interrupted_boundaries, 1, "interrupted boundary count")?;
            active_loop = false;
        } else if duration {
            duration_boundaries = add(duration_boundaries, 1, "turn-duration boundary count")?;
            active_loop = false;
        }

        if kind == "user" && is_human_message(object) && !active_loop {
            active_loop = true;
        }
        if kind == "assistant" {
            process_assistant(
                object,
                path,
                records,
                &mut responses,
                &mut response_usage_records,
                &mut progressive_usage_snapshots,
                &mut duplicate_usage_snapshots,
                &mut usage_without_message_id,
                &mut terminal_response_ids,
                &mut tools_by_id,
                &mut tool_kinds,
            )?;
        }
    }

    if session_ids.len() != 1 {
        return Err(AppError::invalid(format!(
            "Claude source {} must state exactly one sessionId; found {}",
            path.display(),
            session_ids.len()
        )));
    }
    if agent_ids.len() > 1 || sidechain_values.len() > 1 {
        return Err(AppError::invalid(format!(
            "Claude source {} mixes runtime contexts",
            path.display()
        )));
    }
    let agent_id = agent_ids.into_iter().next();
    let mut loops = LoopFacts {
        total: add(duration_boundaries, interrupted_boundaries, "loop count")?,
        interrupted: interrupted_boundaries,
        ended_without_outcome: duration_boundaries,
        ..LoopFacts::default()
    };
    if duration_boundaries == 0 {
        let completed = usize_u64(terminal_response_ids.len())?;
        loops.completed = completed;
        loops.total = add(loops.total, completed, "loop count")?;
        if completed > 0 {
            active_loop = false;
        }
    }
    if active_loop {
        loops.open = 1;
        loops.total = add(loops.total, 1, "loop count")?;
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
    let missing_parent_uuids = parent_uuids.difference(&uuids).cloned().collect::<Vec<_>>();
    if !missing_parent_uuids.is_empty() {
        incomplete.insert("record_tree_has_missing_parents".to_string());
    }
    let unique_response_count = usize_u64(responses.len())?;

    Ok(ParsedClaude {
        session_id: session_ids
            .into_iter()
            .next()
            .ok_or_else(|| AppError::internal("Claude session identity disappeared"))?,
        agent_id,
        is_sidechain: sidechain_values.into_iter().next(),
        source: SourceFacts {
            path: path.display().to_string(),
            sha256: sha256_string(hasher.finalize().as_slice()),
            bytes: source_bytes,
            records,
            terminal_newline,
        },
        runtime_versions: versions,
        records_by_type,
        identified_records: usize_u64(uuids.len())?,
        root_records,
        missing_parent_uuids,
        declared_agents,
        loops,
        responses,
        response_usage_records,
        unique_response_count,
        progressive_usage_snapshots,
        duplicate_usage_snapshots,
        usage_without_message_id,
        tools_by_id,
        tool_kinds,
        incomplete_reasons: incomplete,
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "the outputs expose Claude response snapshot and tool identities independently"
)]
fn process_assistant(
    object: &Map<String, Value>,
    path: &Path,
    record: u64,
    responses: &mut BTreeMap<String, Usage>,
    response_usage_records: &mut u64,
    progressive_usage_snapshots: &mut u64,
    duplicate_usage_snapshots: &mut u64,
    usage_without_message_id: &mut u64,
    terminal_response_ids: &mut BTreeSet<String>,
    tools_by_id: &mut BTreeMap<String, String>,
    tool_kinds: &mut BTreeMap<String, u64>,
) -> Result<(), AppError> {
    let message = object
        .get("message")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            AppError::invalid(format!(
                "invalid Claude assistant message in {} record {record}",
                path.display()
            ))
        })?;
    let message_id = message.get("id").and_then(Value::as_str);
    if let Some(usage) = message.get("usage") {
        *response_usage_records = add(*response_usage_records, 1, "assistant usage record count")?;
        let sample = parse_usage(usage, path, record)?;
        if let Some(id) = message_id {
            match responses.get_mut(id) {
                None => {
                    responses.insert(id.to_string(), sample);
                }
                Some(previous) if previous == &sample => {
                    *duplicate_usage_snapshots = add(
                        *duplicate_usage_snapshots,
                        1,
                        "duplicate usage snapshot count",
                    )?;
                }
                Some(previous) if usage_dominates(&sample, previous) => {
                    *previous = sample;
                    *progressive_usage_snapshots = add(
                        *progressive_usage_snapshots,
                        1,
                        "progressive usage snapshot count",
                    )?;
                }
                Some(previous) if usage_dominates(previous, &sample) => {
                    *progressive_usage_snapshots = add(
                        *progressive_usage_snapshots,
                        1,
                        "progressive usage snapshot count",
                    )?;
                }
                Some(_) => {
                    return Err(AppError::unsupported(format!(
                        "Claude usage snapshots for message {id:?} are not monotonic in {} record {record}",
                        path.display()
                    )));
                }
            }
        } else {
            *usage_without_message_id = add(
                *usage_without_message_id,
                1,
                "usage without message id count",
            )?;
            let _ = sample;
        }
    }
    if matches!(
        message.get("stop_reason").and_then(Value::as_str),
        Some("end_turn" | "stop_sequence")
    ) && let Some(id) = message_id
    {
        terminal_response_ids.insert(id.to_string());
    }
    if let Some(content) = message.get("content").and_then(Value::as_array) {
        for block in content {
            let kind = block
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            if matches!(kind, "tool_use" | "server_tool_use" | "mcp_tool_use") {
                let id = block.get("id").and_then(Value::as_str).map_or_else(
                    || format!("missing:{}:{record}", path.display()),
                    str::to_string,
                );
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string();
                match tools_by_id.get(&id) {
                    Some(existing) if existing != &name => {
                        return Err(AppError::invalid(format!(
                            "Claude tool id {id:?} has conflicting names in {}",
                            path.display()
                        )));
                    }
                    Some(_) => {}
                    None => {
                        tools_by_id.insert(id, name);
                        increment(tool_kinds, kind)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn parse_usage(value: &Value, path: &Path, record: u64) -> Result<Usage, AppError> {
    let object = value.as_object().ok_or_else(|| {
        AppError::invalid(format!(
            "Claude usage in {} record {record} is not an object",
            path.display()
        ))
    })?;
    let raw_input = required_u64(object, "input_tokens", path, record)?;
    let cached = optional_u64(object, "cache_read_input_tokens", path, record)?.unwrap_or(0);
    let cache_write =
        optional_u64(object, "cache_creation_input_tokens", path, record)?.unwrap_or(0);
    let input = add(
        add(raw_input, cached, "normalized input tokens")?,
        cache_write,
        "normalized input tokens",
    )?;
    let output = required_u64(object, "output_tokens", path, record)?;
    let reasoning = object
        .get("output_tokens_details")
        .and_then(Value::as_object)
        .and_then(|details| details.get("thinking_tokens"))
        .map(|value| {
            value.as_u64().ok_or_else(|| {
                AppError::invalid(format!(
                    "Claude thinking_tokens in {} record {record} is not a non-negative integer",
                    path.display()
                ))
            })
        })
        .transpose()?;
    if reasoning.is_some_and(|reasoning| reasoning > output) {
        return Err(AppError::invalid(format!(
            "Claude thinking_tokens exceeds output_tokens in {} record {record}",
            path.display()
        )));
    }
    Ok(Usage {
        input,
        cached: Some(cached),
        cache_write: Some(cache_write),
        output,
        reasoning,
    })
}

fn is_human_message(object: &Map<String, Value>) -> bool {
    if object.get("isMeta").and_then(Value::as_bool) == Some(true) {
        return false;
    }
    let Some(content) = object
        .get("message")
        .and_then(Value::as_object)
        .and_then(|message| message.get("content"))
    else {
        return false;
    };
    match content {
        Value::String(text) => !text.is_empty(),
        Value::Array(blocks) => blocks.iter().any(|block| {
            block.get("type").and_then(Value::as_str) == Some("text")
                && block
                    .get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|text| !text.is_empty())
        }),
        _ => false,
    }
}

fn collect_agent_ids(value: &Value, ids: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            if let Some(id) = object.get("agentId").and_then(Value::as_str) {
                ids.insert(id.to_string());
            }
            for nested in object.values() {
                collect_agent_ids(nested, ids);
            }
        }
        Value::Array(values) => {
            for nested in values {
                collect_agent_ids(nested, ids);
            }
        }
        _ => {}
    }
}

fn usage_dominates(left: &Usage, right: &Usage) -> bool {
    left.input >= right.input
        && optional_dominates(left.cached, right.cached)
        && optional_dominates(left.cache_write, right.cache_write)
        && left.output >= right.output
        && optional_dominates(left.reasoning, right.reasoning)
}

fn optional_dominates(left: Option<u64>, right: Option<u64>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left >= right,
        (Some(_) | None, None) => true,
        (None, Some(_)) => false,
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

fn add_loops(total: &mut LoopFacts, value: &LoopFacts) -> Result<(), AppError> {
    total.total = add(total.total, value.total, "loop count")?;
    total.completed = add(total.completed, value.completed, "completed loop count")?;
    total.failed = add(total.failed, value.failed, "failed loop count")?;
    total.interrupted = add(
        total.interrupted,
        value.interrupted,
        "interrupted loop count",
    )?;
    total.ended_without_outcome = add(
        total.ended_without_outcome,
        value.ended_without_outcome,
        "unknown-outcome loop count",
    )?;
    total.open = add(total.open, value.open, "open loop count")?;
    Ok(())
}

fn add_counts(
    total: &mut BTreeMap<String, u64>,
    values: &BTreeMap<String, u64>,
    field: &str,
) -> Result<(), AppError> {
    for (key, count) in values {
        let current = total.entry(key.clone()).or_default();
        *current = add(*current, *count, field)?;
    }
    Ok(())
}

fn increment(values: &mut BTreeMap<String, u64>, key: &str) -> Result<(), AppError> {
    let current = values.entry(key.to_string()).or_default();
    *current = add(*current, 1, "categorized count")?;
    Ok(())
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
    u64::try_from(value).map_err(|_| AppError::internal("count does not fit in u64"))
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
    use std::path::Path;

    use serde_json::Value;
    use serde_json::json;
    use tempfile::TempDir;

    use super::inspect;

    #[test]
    fn deduplicates_claude_response_usage_and_aggregates_child_context() {
        let temp = TempDir::new().expect("temporary directory should be created");
        let root = temp.path().join("root.jsonl");
        let child = temp.path().join("child.jsonl");
        write_jsonl(
            &root,
            &[
                json!({"type":"user","uuid":"u1","parentUuid":null,"sessionId":"session","isSidechain":false,"promptId":"p1","version":"2.1.1","message":{"role":"user","content":"do it"}}),
                json!({"type":"assistant","uuid":"a1","parentUuid":"u1","sessionId":"session","isSidechain":false,"version":"2.1.1","message":{"id":"m1","role":"assistant","stop_reason":"end_turn","usage":{"input_tokens":2,"cache_read_input_tokens":3,"cache_creation_input_tokens":5,"output_tokens":1},"content":[{"type":"thinking","thinking":"x"}]}}),
                json!({"type":"assistant","uuid":"a2","parentUuid":"a1","sessionId":"session","isSidechain":false,"version":"2.1.1","message":{"id":"m1","role":"assistant","stop_reason":"end_turn","usage":{"input_tokens":2,"cache_read_input_tokens":3,"cache_creation_input_tokens":5,"output_tokens":7,"output_tokens_details":{"thinking_tokens":2}},"content":[{"type":"tool_use","id":"tool-1","name":"Agent"}]}}),
                json!({"type":"user","uuid":"r1","parentUuid":"a2","sessionId":"session","isSidechain":false,"promptId":"p1","version":"2.1.1","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tool-1","content":"launched"}]},"toolUseResult":{"agentId":"child-agent","status":"async_launched"}}),
                json!({"type":"system","subtype":"turn_duration","uuid":"s1","parentUuid":"r1","sessionId":"session","isSidechain":false,"version":"2.1.1"}),
            ],
        );
        write_jsonl(
            &child,
            &[
                json!({"type":"user","uuid":"cu1","parentUuid":null,"sessionId":"session","agentId":"child-agent","isSidechain":true,"promptId":"cp1","version":"2.1.1","message":{"role":"user","content":"child work"}}),
                json!({"type":"assistant","uuid":"ca1","parentUuid":"cu1","sessionId":"session","agentId":"child-agent","isSidechain":true,"version":"2.1.1","message":{"id":"cm1","role":"assistant","stop_reason":"end_turn","usage":{"input_tokens":11,"cache_read_input_tokens":13,"cache_creation_input_tokens":17,"output_tokens":19,"output_tokens_details":{"thinking_tokens":5}},"content":[{"type":"tool_use","id":"tool-2","name":"Read"}]}}),
            ],
        );

        let report = inspect(&[root, child]).expect("valid Claude Session should be inspected");
        assert!(report.complete);
        assert_eq!(report.uniform.session.source_count, 2);
        assert_eq!(report.uniform.session.loops.total, 2);
        assert_eq!(report.uniform.session.loops.ended_without_outcome, 1);
        assert_eq!(report.uniform.session.loops.completed, 1);
        let usage = report
            .uniform
            .session
            .usage
            .values
            .expect("usage should be complete");
        assert_eq!(usage.input, 51);
        assert_eq!(usage.output, 26);
        assert_eq!(usage.reasoning, Some(7));
        assert_eq!(report.uniform.session.items.agent_tool_calls.total, 2);
        assert_eq!(
            report.runtime_specific.facts["usage_accounting"]["progressive_snapshots"],
            1
        );
    }

    fn write_jsonl(path: &Path, records: &[Value]) {
        let contents = records
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        fs::write(path, contents).expect("fixture should be written");
    }
}
