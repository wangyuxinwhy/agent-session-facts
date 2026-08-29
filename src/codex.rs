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

#[allow(
    clippy::too_many_lines,
    reason = "session assembly keeps coverage and checked aggregation visible in one audit path"
)]
pub(crate) fn inspect(paths: &[PathBuf]) -> Result<Report, AppError> {
    if paths.is_empty() {
        return Err(AppError::invalid("at least one rollout file is required"));
    }
    if paths.len() > MAX_SOURCES {
        return Err(AppError::invalid(format!(
            "at most {MAX_SOURCES} rollout files may be inspected at once"
        )));
    }

    let mut parsed = Vec::with_capacity(paths.len());
    let mut canonical_paths = BTreeSet::new();
    for path in paths {
        let canonical = std::fs::canonicalize(path).map_err(|error| {
            AppError::io(format!(
                "cannot resolve rollout {}: {error}",
                path.display()
            ))
        })?;
        if !canonical_paths.insert(canonical.clone()) {
            return Err(AppError::invalid(format!(
                "rollout was supplied more than once: {}",
                canonical.display()
            )));
        }
        parsed.push(parse_source(&canonical)?);
    }

    let session_id = parsed
        .first()
        .map(|source| source.header.session_id.clone())
        .ok_or_else(|| AppError::internal("parsed source set is unexpectedly empty"))?;
    for source in &parsed {
        if source.header.session_id != session_id {
            return Err(AppError::invalid(format!(
                "rollouts belong to different native Codex sessions: expected {session_id}, found {} in {}",
                source.header.session_id, source.source.path
            )));
        }
    }

    let mut thread_ids = BTreeSet::new();
    for source in &parsed {
        if !thread_ids.insert(source.header.thread_id.clone()) {
            return Err(AppError::unsupported(format!(
                "multiple rollouts for Codex thread {} require history_base lineage accounting, which is not yet supported",
                source.header.thread_id
            )));
        }
    }

    let declared_child_threads: BTreeSet<_> = parsed
        .iter()
        .flat_map(|source| source.declared_child_threads.iter().cloned())
        .collect();
    let missing_declared_children: Vec<_> = declared_child_threads
        .difference(&thread_ids)
        .cloned()
        .collect();

    let mut incomplete = BTreeSet::new();
    for source in &parsed {
        incomplete.extend(source.incomplete_reasons.iter().cloned());
        if let Some(parent) = &source.header.parent_thread_id
            && !thread_ids.contains(parent)
        {
            incomplete.insert(format!("missing_parent_thread:{parent}"));
        }
    }
    for child in &missing_declared_children {
        incomplete.insert(format!("missing_runtime_declared_child_thread:{child}"));
    }

    let mut session_loops = LoopFacts::default();
    let mut session_tools = ToolCallFacts::default();
    let mut session_tool_kinds = BTreeMap::new();
    let mut known_usage = Usage::default();
    let mut usage_threads = 0_u64;
    let mut reported_samples = 0_u64;
    let mut cumulative_updates = 0_u64;
    let mut duplicate_snapshots = 0_u64;
    let mut counter_resets = 0_u64;
    let mut runtime_versions = BTreeSet::new();
    let mut contexts = Vec::with_capacity(parsed.len());
    let mut source_details = Vec::with_capacity(parsed.len());
    let mut sources = Vec::with_capacity(parsed.len());

    for source in parsed {
        add_loops(&mut session_loops, &source.loops)?;
        add_tools(&mut session_tools, &source.tool_calls)?;
        add_counts(
            &mut session_tool_kinds,
            &source.tool_call_kinds,
            "tool kind total",
        )?;
        reported_samples = checked_add(reported_samples, source.reported_samples, "samples")?;
        cumulative_updates = checked_add(
            cumulative_updates,
            source.cumulative_updates,
            "usage updates",
        )?;
        duplicate_snapshots = checked_add(
            duplicate_snapshots,
            source.duplicate_snapshots,
            "duplicate snapshots",
        )?;
        counter_resets = checked_add(counter_resets, source.counter_resets, "counter resets")?;
        if let Some(usage) = &source.token_usage {
            usage_threads = checked_add(usage_threads, 1, "usage thread count")?;
            add_usage(&mut known_usage, usage)?;
        }
        runtime_versions.insert(source.header.cli_version.clone());

        let context_usage = usage_facts(
            1,
            u64::from(source.token_usage.is_some()),
            source.token_usage.clone().unwrap_or_default(),
        );
        contexts.push(serde_json::json!({
            "thread_id": source.header.thread_id,
            "parent_thread_id": source.header.parent_thread_id,
            "forked_from_thread_id": source.header.forked_from_thread_id,
            "agent_path": source.header.agent_path,
            "cli_version": source.header.cli_version,
            "own_history_start_ordinal": source.header.own_start_ordinal,
            "loops": source.loops,
            "usage": context_usage,
            "tool_calls": {
                "total": source.tool_calls.total,
                "by_name": source.tool_calls.by_name,
                "by_kind": source.tool_call_kinds,
            },
            "usage_accounting": {
                "reported_samples": source.reported_samples,
                "cumulative_updates": source.cumulative_updates,
                "duplicate_snapshots": source.duplicate_snapshots,
                "counter_resets": source.counter_resets,
            }
        }));
        source_details.push(serde_json::json!({
            "path": source.source.path,
            "thread_id": source.source_thread_id,
            "included_records": source.included_records,
            "first_ordinal": source.first_ordinal,
            "last_ordinal": source.last_ordinal,
        }));
        sources.push(source.source);
    }

    sources.sort_by(|left, right| left.path.cmp(&right.path));
    contexts.sort_by(|left, right| left["thread_id"].as_str().cmp(&right["thread_id"].as_str()));
    source_details.sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
    let thread_count = u64::try_from(contexts.len())
        .map_err(|_| AppError::internal("thread count does not fit in u64"))?;
    let source_count = u64::try_from(sources.len())
        .map_err(|_| AppError::internal("rollout count does not fit in u64"))?;
    let declared_child_count = u64::try_from(declared_child_threads.len())
        .map_err(|_| AppError::internal("declared child thread count does not fit in u64"))?;
    if usage_threads != thread_count {
        incomplete.insert("token_usage_not_reported_by_every_thread".to_string());
    }
    if session_loops.open > 0 {
        incomplete.insert("open_loops".to_string());
    }

    let token_usage = usage_facts(thread_count, usage_threads, known_usage);
    let incomplete_reasons: Vec<_> = incomplete.into_iter().collect();
    Ok(Report {
        schema_version: 1,
        kind: "agent_session_facts.report",
        runtime: "codex",
        complete: incomplete_reasons.is_empty(),
        incomplete_reasons,
        uniform: UniformFacts {
            schema: "agent_session_facts.uniform@1",
            session: SessionFacts {
                native_id: session_id,
                source_count,
                runtime_versions: runtime_versions.into_iter().collect(),
                source_coverage: SourceCoverageFacts {
                    status: if missing_declared_children.is_empty() {
                        "complete"
                    } else {
                        "incomplete"
                    },
                    basis: "supplied_sources_and_runtime_declared_child_sessions",
                    supplied_sources: source_count,
                    runtime_declared_child_sessions: declared_child_count,
                    missing_runtime_declared_child_sessions: missing_declared_children,
                },
                loops: session_loops,
                usage: token_usage,
                items: ItemFacts {
                    agent_tool_calls: session_tools,
                },
                model_cost: ModelCostFacts {
                    status: AvailabilityStatus::Unavailable,
                    amount_usd: None,
                    basis: None,
                    reason: Some("not_recorded_in_codex_rollout"),
                },
            },
            sources,
        },
        runtime_specific: RuntimeSpecificFacts {
            schema: "agent_session_facts.codex@1",
            facts: serde_json::json!({
                "thread_count": thread_count,
                "threads": contexts,
                "sources": source_details,
                "tool_calls_by_kind": session_tool_kinds,
                "usage_accounting": {
                    "reported_samples": reported_samples,
                    "cumulative_updates": cumulative_updates,
                    "duplicate_snapshots": duplicate_snapshots,
                    "counter_resets": counter_resets,
                }
            }),
        },
    })
}

struct ParsedSource {
    header: Header,
    source: SourceFacts,
    source_thread_id: String,
    included_records: u64,
    first_ordinal: Option<u64>,
    last_ordinal: Option<u64>,
    loops: LoopFacts,
    token_usage: Option<Usage>,
    reported_samples: u64,
    cumulative_updates: u64,
    duplicate_snapshots: u64,
    counter_resets: u64,
    tool_calls: ToolCallFacts,
    tool_call_kinds: BTreeMap<String, u64>,
    declared_child_threads: BTreeSet<String>,
    incomplete_reasons: BTreeSet<String>,
}

#[derive(Clone)]
struct Header {
    session_id: String,
    thread_id: String,
    parent_thread_id: Option<String>,
    forked_from_thread_id: Option<String>,
    agent_path: Option<String>,
    cli_version: String,
    own_start_ordinal: u64,
    paginated: bool,
}

#[derive(Clone, Copy)]
enum TurnState {
    Open,
    Completed,
    Failed,
    Aborted,
}

#[allow(
    clippy::too_many_lines,
    reason = "the streaming parser keeps hashing, validation, and folding in exact source order"
)]
fn parse_source(path: &Path) -> Result<ParsedSource, AppError> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        AppError::io(format!(
            "cannot inspect rollout {}: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(AppError::invalid(format!(
            "rollout is not a regular file: {}",
            path.display()
        )));
    }
    if metadata.len() > MAX_SOURCE_BYTES {
        return Err(AppError::invalid(format!(
            "rollout exceeds the {MAX_SOURCE_BYTES}-byte source limit: {}",
            path.display()
        )));
    }

    let file = File::open(path).map_err(|error| {
        AppError::io(format!("cannot open rollout {}: {error}", path.display()))
    })?;
    let mut reader = BufReader::new(file);
    let mut buffer = Vec::new();
    let mut hasher = Sha256::new();
    let mut source_bytes = 0_u64;
    let mut header = None;
    let mut record_count = 0_u64;
    let mut own_records = 0_u64;
    let mut terminal_newline = false;
    let mut first_ordinal = None;
    let mut last_ordinal = None;
    let mut previous_ordinal = None;
    let mut turns = BTreeMap::new();
    let mut completed_usage_segments = Usage::default();
    let mut previous_usage = None;
    let mut reported_samples = 0_u64;
    let mut cumulative_updates = 0_u64;
    let mut duplicate_snapshots = 0_u64;
    let mut counter_resets = 0_u64;
    let mut tool_calls = ToolCallFacts::default();
    let mut tool_call_kinds = BTreeMap::new();
    let mut declared_child_threads = BTreeSet::new();
    let mut incomplete = BTreeSet::new();

    loop {
        buffer.clear();
        let bytes = reader.read_until(b'\n', &mut buffer).map_err(|error| {
            AppError::io(format!("cannot read rollout {}: {error}", path.display()))
        })?;
        if bytes == 0 {
            break;
        }
        if bytes > MAX_RECORD_BYTES {
            return Err(AppError::invalid(format!(
                "rollout record {} in {} exceeds the {MAX_RECORD_BYTES}-byte record limit",
                record_count + 1,
                path.display()
            )));
        }
        source_bytes = checked_add(
            source_bytes,
            u64::try_from(bytes)
                .map_err(|_| AppError::internal("record byte count does not fit in u64"))?,
            "source byte count",
        )?;
        if source_bytes > MAX_SOURCE_BYTES {
            return Err(AppError::invalid(format!(
                "rollout exceeds the {MAX_SOURCE_BYTES}-byte source limit while reading: {}",
                path.display()
            )));
        }
        hasher.update(&buffer);
        record_count = checked_add(record_count, 1, "record count")?;
        terminal_newline = buffer.last() == Some(&b'\n');
        if terminal_newline {
            buffer.pop();
            if buffer.last() == Some(&b'\r') {
                buffer.pop();
            }
        }
        if buffer.is_empty() {
            return Err(AppError::invalid(format!(
                "empty JSONL record {record_count} in {}",
                path.display()
            )));
        }
        let value: Value = serde_json::from_slice(&buffer).map_err(|error| {
            AppError::invalid(format!(
                "invalid JSON in rollout {} at record {record_count}: {error}",
                path.display()
            ))
        })?;
        let object = value.as_object().ok_or_else(|| {
            AppError::invalid(format!(
                "rollout {} record {record_count} is not a JSON object",
                path.display()
            ))
        })?;

        if header.is_none() {
            header = Some(parse_header(object, path)?);
        }
        let header_ref = header
            .as_ref()
            .ok_or_else(|| AppError::internal("rollout header was not retained"))?;
        let ordinal = record_ordinal(object, header_ref, record_count, path)?;
        if let Some(previous) = previous_ordinal
            && ordinal <= previous
        {
            return Err(AppError::invalid(format!(
                "rollout {} ordinals are not strictly increasing at {ordinal}",
                path.display()
            )));
        }
        previous_ordinal = Some(ordinal);
        first_ordinal.get_or_insert(ordinal);
        last_ordinal = Some(ordinal);
        if ordinal < header_ref.own_start_ordinal {
            continue;
        }
        own_records = checked_add(own_records, 1, "owned record count")?;
        process_record(
            object,
            path,
            record_count,
            &mut turns,
            &mut completed_usage_segments,
            &mut previous_usage,
            &mut reported_samples,
            &mut cumulative_updates,
            &mut duplicate_snapshots,
            &mut counter_resets,
            &mut tool_calls,
            &mut tool_call_kinds,
            &mut declared_child_threads,
            &mut incomplete,
        )?;
    }

    let final_source_len = reader
        .get_ref()
        .metadata()
        .map_err(|error| {
            AppError::io(format!(
                "cannot recheck rollout {} after reading: {error}",
                path.display()
            ))
        })?
        .len();
    let header =
        header.ok_or_else(|| AppError::invalid(format!("rollout is empty: {}", path.display())))?;
    if !terminal_newline {
        incomplete.insert(format!(
            "source_missing_terminal_newline:{}",
            path.display()
        ));
    }
    if header.parent_thread_id.is_some()
        && last_ordinal.is_none_or(|ordinal| ordinal < header.own_start_ordinal)
    {
        incomplete.insert(format!(
            "subagent_history_prefix_incomplete:{}",
            path.display()
        ));
    }
    if source_bytes != metadata.len() || source_bytes != final_source_len {
        incomplete.insert(format!("source_changed_during_read:{}", path.display()));
    }
    let loop_facts = finish_loops(&turns)?;
    if loop_facts.open > 0 {
        incomplete.insert(format!("open_loops_in_thread:{}", header.thread_id));
    }
    let source = SourceFacts {
        path: path.display().to_string(),
        sha256: sha256_string(hasher.finalize().as_slice()),
        bytes: source_bytes,
        records: record_count,
        terminal_newline,
    };
    let token_usage = if let Some(current_segment) = previous_usage {
        add_usage(&mut completed_usage_segments, &current_segment)?;
        Some(completed_usage_segments)
    } else {
        None
    };
    Ok(ParsedSource {
        source,
        source_thread_id: header.thread_id.clone(),
        included_records: own_records,
        first_ordinal,
        last_ordinal,
        loops: loop_facts,
        token_usage,
        reported_samples,
        cumulative_updates,
        duplicate_snapshots,
        counter_resets,
        tool_calls,
        tool_call_kinds,
        declared_child_threads,
        incomplete_reasons: incomplete,
        header,
    })
}

fn parse_header(object: &Map<String, Value>, path: &Path) -> Result<Header, AppError> {
    let record_type = string_value(object, "type");
    if record_type != Some("session_meta") {
        let detail = if record_type.is_some_and(|value| value.contains('.')) {
            "the 'codex exec --json' event stream is a different format"
        } else {
            "the first record must be session_meta"
        };
        return Err(AppError::invalid(format!(
            "{} is not a Codex on-disk rollout: {detail}",
            path.display()
        )));
    }
    let payload = object_at(object, "payload", path, 1)?;
    if payload
        .get("history_base")
        .is_some_and(|value| !value.is_null())
    {
        return Err(AppError::unsupported(format!(
            "{} uses Codex history_base rollout lineage; exact revert accounting is not yet supported",
            path.display()
        )));
    }
    let session_id = required_string(payload, "session_id", path, 1)?;
    let thread_id = required_string(payload, "id", path, 1)?;
    let cli_version = required_string(payload, "cli_version", path, 1)?;
    let parent_thread_id = optional_string(payload, "parent_thread_id", path, 1)?;
    let forked_from_thread_id = optional_string(payload, "forked_from_id", path, 1)?;
    let agent_path = optional_string(payload, "agent_path", path, 1)?;
    let own_start = optional_u64(payload, "subagent_history_start_ordinal", path, 1)?;
    if parent_thread_id.is_some() && own_start.is_none() {
        return Err(AppError::unsupported(format!(
            "{} is a child Codex thread without subagent_history_start_ordinal; inherited history cannot be excluded safely",
            path.display()
        )));
    }
    let history_mode = string_value(payload, "history_mode").unwrap_or("legacy");
    let paginated = match history_mode {
        "paginated" => true,
        "legacy" => false,
        other => {
            return Err(AppError::unsupported(format!(
                "{} uses unknown Codex history_mode {other:?}",
                path.display()
            )));
        }
    };
    Ok(Header {
        session_id,
        thread_id,
        parent_thread_id,
        forked_from_thread_id,
        agent_path,
        cli_version,
        own_start_ordinal: own_start.unwrap_or(0),
        paginated,
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "all mutable values belong to one streaming record fold"
)]
fn process_record(
    object: &Map<String, Value>,
    path: &Path,
    record: u64,
    turns: &mut BTreeMap<String, TurnState>,
    completed_usage_segments: &mut Usage,
    previous_usage: &mut Option<Usage>,
    reported_samples: &mut u64,
    cumulative_updates: &mut u64,
    duplicate_snapshots: &mut u64,
    counter_resets: &mut u64,
    tool_calls: &mut ToolCallFacts,
    tool_call_kinds: &mut BTreeMap<String, u64>,
    declared_child_threads: &mut BTreeSet<String>,
    incomplete: &mut BTreeSet<String>,
) -> Result<(), AppError> {
    let record_type = string_value(object, "type");
    let Some(payload) = object.get("payload").and_then(Value::as_object) else {
        return Ok(());
    };
    match (record_type, string_value(payload, "type")) {
        (Some("event_msg"), Some("task_started" | "turn_started")) => {
            let turn_id = required_string(payload, "turn_id", path, record)?;
            if turns.insert(turn_id.clone(), TurnState::Open).is_some() {
                incomplete.insert(format!("duplicate_turn_start:{turn_id}"));
            }
        }
        (Some("event_msg"), Some("task_complete" | "turn_complete")) => {
            let turn_id = required_string(payload, "turn_id", path, record)?;
            let state = if payload.get("error").is_some_and(|value| !value.is_null()) {
                TurnState::Failed
            } else {
                TurnState::Completed
            };
            finish_turn(turns, &turn_id, state, incomplete);
        }
        (Some("event_msg"), Some("turn_aborted")) => {
            let Some(turn_id) = optional_string(payload, "turn_id", path, record)? else {
                incomplete.insert(format!(
                    "turn_aborted_without_id:{}:{record}",
                    path.display()
                ));
                return Ok(());
            };
            finish_turn(turns, &turn_id, TurnState::Aborted, incomplete);
        }
        (Some("event_msg"), Some("token_count")) => {
            let Some(info) = payload.get("info").and_then(Value::as_object) else {
                return Ok(());
            };
            let total = object_at(info, "total_token_usage", path, record)?;
            let usage = parse_usage(total, path, record)?;
            *reported_samples = checked_add(*reported_samples, 1, "reported samples")?;
            if previous_usage.as_ref() == Some(&usage) {
                *duplicate_snapshots = checked_add(*duplicate_snapshots, 1, "duplicate snapshots")?;
            } else {
                if let Some(previous) = previous_usage.as_ref() {
                    if usage_total(&usage)? < usage_total(previous)? {
                        add_usage(completed_usage_segments, previous)?;
                        *counter_resets = checked_add(*counter_resets, 1, "counter resets")?;
                    } else if !usage_is_monotonic(previous, &usage) {
                        return Err(AppError::unsupported(format!(
                            "cumulative token usage fields are inconsistent in {} record {record}",
                            path.display()
                        )));
                    }
                }
                *cumulative_updates = checked_add(*cumulative_updates, 1, "cumulative updates")?;
                *previous_usage = Some(usage.clone());
            }
        }
        (Some("event_msg"), Some("item_completed")) => {
            if let Some(item) = payload.get("item").and_then(Value::as_object)
                && matches!(
                    string_value(item, "type"),
                    Some("SubAgentActivity" | "sub_agent_activity")
                )
                && string_value(item, "kind") == Some("started")
            {
                let child = required_string(item, "agent_thread_id", path, record)?;
                declared_child_threads.insert(child);
            }
        }
        (Some("response_item"), Some(kind)) => {
            if let Some(name) = tool_call_name(kind, payload) {
                add_tool_call(tool_calls, tool_call_kinds, kind, name)?;
            } else if kind.ends_with("_call") {
                add_tool_call(tool_calls, tool_call_kinds, kind, kind)?;
                incomplete.insert(format!("unknown_tool_call_kind:{kind}"));
            }
        }
        _ => {}
    }
    Ok(())
}

fn finish_turn(
    turns: &mut BTreeMap<String, TurnState>,
    turn_id: &str,
    terminal: TurnState,
    incomplete: &mut BTreeSet<String>,
) {
    match turns.get_mut(turn_id) {
        Some(state @ TurnState::Open) => *state = terminal,
        Some(_) => {
            incomplete.insert(format!("duplicate_turn_terminal:{turn_id}"));
        }
        None => {
            incomplete.insert(format!("turn_terminal_without_start:{turn_id}"));
        }
    }
}

fn finish_loops(turns: &BTreeMap<String, TurnState>) -> Result<LoopFacts, AppError> {
    let mut facts = LoopFacts {
        total: u64::try_from(turns.len())
            .map_err(|_| AppError::internal("loop count does not fit in u64"))?,
        ..LoopFacts::default()
    };
    for state in turns.values() {
        match state {
            TurnState::Open => facts.open += 1,
            TurnState::Completed => facts.completed += 1,
            TurnState::Failed => facts.failed += 1,
            TurnState::Aborted => facts.interrupted += 1,
        }
    }
    Ok(facts)
}

fn tool_call_name<'a>(kind: &'a str, payload: &'a Map<String, Value>) -> Option<&'a str> {
    match kind {
        "function_call" | "custom_tool_call" => string_value(payload, "name"),
        "local_shell_call" => Some("local_shell"),
        "tool_search_call" => Some("tool_search"),
        "web_search_call" => Some("web_search"),
        "image_generation_call" => Some("image_generation"),
        _ => None,
    }
}

fn add_tool_call(
    facts: &mut ToolCallFacts,
    kinds: &mut BTreeMap<String, u64>,
    kind: &str,
    name: &str,
) -> Result<(), AppError> {
    facts.total = checked_add(facts.total, 1, "tool call count")?;
    let kind_count = kinds.entry(kind.to_string()).or_default();
    *kind_count = checked_add(*kind_count, 1, "tool call kind count")?;
    let name_count = facts.by_name.entry(name.to_string()).or_default();
    *name_count = checked_add(*name_count, 1, "tool call name count")?;
    Ok(())
}

fn parse_usage(object: &Map<String, Value>, path: &Path, record: u64) -> Result<Usage, AppError> {
    let input = required_u64(object, "input_tokens", path, record)?;
    let cached = required_u64(object, "cached_input_tokens", path, record)?;
    let cache_write = optional_u64(object, "cache_write_input_tokens", path, record)?.unwrap_or(0);
    let output = required_u64(object, "output_tokens", path, record)?;
    let reasoning = required_u64(object, "reasoning_output_tokens", path, record)?;
    let total = required_u64(object, "total_tokens", path, record)?;
    if cached > input {
        return Err(AppError::invalid(format!(
            "cached_input_tokens exceeds input_tokens in {} record {record}",
            path.display()
        )));
    }
    if cache_write > input {
        return Err(AppError::invalid(format!(
            "cache_write_input_tokens exceeds input_tokens in {} record {record}",
            path.display()
        )));
    }
    if cached
        .checked_add(cache_write)
        .is_none_or(|known| known > input)
    {
        return Err(AppError::invalid(format!(
            "cached_input_tokens + cache_write_input_tokens exceeds input_tokens in {} record {record}",
            path.display()
        )));
    }
    if reasoning > output {
        return Err(AppError::invalid(format!(
            "reasoning_output_tokens exceeds output_tokens in {} record {record}",
            path.display()
        )));
    }
    if input.checked_add(output) != Some(total) {
        return Err(AppError::invalid(format!(
            "total_tokens does not equal input_tokens + output_tokens in {} record {record}",
            path.display()
        )));
    }
    Ok(Usage {
        input,
        cached: Some(cached),
        cache_write: Some(cache_write),
        output,
        reasoning: Some(reasoning),
    })
}

fn usage_is_monotonic(previous: &Usage, current: &Usage) -> bool {
    current.input >= previous.input
        && optional_usage_is_monotonic(previous.cached, current.cached)
        && optional_usage_is_monotonic(previous.cache_write, current.cache_write)
        && current.output >= previous.output
        && optional_usage_is_monotonic(previous.reasoning, current.reasoning)
}

fn usage_facts(thread_count: u64, threads_with_usage: u64, usage: Usage) -> UsageFacts {
    let (status, values, known_values) = if threads_with_usage == 0 {
        (AvailabilityStatus::Unavailable, None, None)
    } else if threads_with_usage == thread_count {
        (AvailabilityStatus::Available, Some(usage), None)
    } else {
        (AvailabilityStatus::Partial, None, Some(usage))
    };
    UsageFacts {
        status,
        values,
        known_values,
    }
}

fn add_loops(total: &mut LoopFacts, value: &LoopFacts) -> Result<(), AppError> {
    total.total = checked_add(total.total, value.total, "turn total")?;
    total.completed = checked_add(total.completed, value.completed, "completed turns")?;
    total.failed = checked_add(total.failed, value.failed, "failed turns")?;
    total.interrupted = checked_add(total.interrupted, value.interrupted, "interrupted loops")?;
    total.ended_without_outcome = checked_add(
        total.ended_without_outcome,
        value.ended_without_outcome,
        "loops ended without outcome",
    )?;
    total.open = checked_add(total.open, value.open, "open turns")?;
    Ok(())
}

fn add_tools(total: &mut ToolCallFacts, value: &ToolCallFacts) -> Result<(), AppError> {
    total.total = checked_add(total.total, value.total, "tool call total")?;
    for (name, count) in &value.by_name {
        let current = total.by_name.entry(name.clone()).or_default();
        *current = checked_add(*current, *count, "tool name total")?;
    }
    Ok(())
}

fn usage_total(value: &Usage) -> Result<u64, AppError> {
    checked_add(value.input, value.output, "total tokens")
}

fn add_usage(total: &mut Usage, value: &Usage) -> Result<(), AppError> {
    total.input = checked_add(total.input, value.input, "input tokens")?;
    total.cached = add_optional_usage(total.cached, value.cached, "cached input tokens")?;
    total.cache_write = add_optional_usage(
        total.cache_write,
        value.cache_write,
        "cache-write input tokens",
    )?;
    total.output = checked_add(total.output, value.output, "output tokens")?;
    total.reasoning =
        add_optional_usage(total.reasoning, value.reasoning, "reasoning output tokens")?;
    Ok(())
}

fn optional_usage_is_monotonic(previous: Option<u64>, current: Option<u64>) -> bool {
    match (previous, current) {
        (Some(previous), Some(current)) => current >= previous,
        (None, None | Some(_)) => true,
        (Some(_), None) => false,
    }
}

fn add_optional_usage(
    total: Option<u64>,
    value: Option<u64>,
    field: &str,
) -> Result<Option<u64>, AppError> {
    match (total, value) {
        (Some(total), Some(value)) => checked_add(total, value, field).map(Some),
        (None, Some(value)) => Ok(Some(value)),
        (_, None) => Ok(None),
    }
}

fn add_counts(
    total: &mut BTreeMap<String, u64>,
    value: &BTreeMap<String, u64>,
    field: &str,
) -> Result<(), AppError> {
    for (name, count) in value {
        let current = total.entry(name.clone()).or_default();
        *current = checked_add(*current, *count, field)?;
    }
    Ok(())
}

fn record_ordinal(
    object: &Map<String, Value>,
    header: &Header,
    record: u64,
    path: &Path,
) -> Result<u64, AppError> {
    match object.get("ordinal") {
        Some(value) => value.as_u64().ok_or_else(|| {
            AppError::invalid(format!(
                "ordinal in {} record {record} is not a non-negative integer",
                path.display()
            ))
        }),
        None if header.paginated => Err(AppError::invalid(format!(
            "paginated rollout {} record {record} is missing ordinal",
            path.display()
        ))),
        None => Ok(record - 1),
    }
}

fn object_at<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    path: &Path,
    record: u64,
) -> Result<&'a Map<String, Value>, AppError> {
    object.get(field).and_then(Value::as_object).ok_or_else(|| {
        AppError::invalid(format!(
            "field {field:?} in {} record {record} is not an object",
            path.display()
        ))
    })
}

fn required_string(
    object: &Map<String, Value>,
    field: &str,
    path: &Path,
    record: u64,
) -> Result<String, AppError> {
    let value = string_value(object, field).ok_or_else(|| {
        AppError::invalid(format!(
            "field {field:?} in {} record {record} is missing or not a string",
            path.display()
        ))
    })?;
    if value.is_empty() {
        return Err(AppError::invalid(format!(
            "field {field:?} in {} record {record} is empty",
            path.display()
        )));
    }
    Ok(value.to_string())
}

fn optional_string(
    object: &Map<String, Value>,
    field: &str,
    path: &Path,
    record: u64,
) -> Result<Option<String>, AppError> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value.clone())),
        Some(_) => Err(AppError::invalid(format!(
            "field {field:?} in {} record {record} is not a non-empty string or null",
            path.display()
        ))),
    }
}

fn string_value<'a>(object: &'a Map<String, Value>, field: &str) -> Option<&'a str> {
    object.get(field).and_then(Value::as_str)
}

fn required_u64(
    object: &Map<String, Value>,
    field: &str,
    path: &Path,
    record: u64,
) -> Result<u64, AppError> {
    optional_u64(object, field, path, record)?.ok_or_else(|| {
        AppError::invalid(format!(
            "field {field:?} in {} record {record} is missing",
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

fn checked_add(left: u64, right: u64, what: &str) -> Result<u64, AppError> {
    left.checked_add(right)
        .ok_or_else(|| AppError::invalid(format!("{what} overflowed u64")))
}

fn sha256_string(digest: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity("sha256:".len() + digest.len() * 2);
    encoded.push_str("sha256:");
    for byte in digest {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use serde_json::Value;
    use serde_json::json;
    use tempfile::TempDir;

    use super::inspect;

    fn write_jsonl(path: &Path, records: &[Value], terminal_newline: bool) {
        let mut contents = records
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        if terminal_newline {
            contents.push('\n');
        }
        fs::write(path, contents).expect("fixture rollout should be writable");
    }

    fn header(
        ordinal: u64,
        session: &str,
        thread: &str,
        parent: Option<&str>,
        own_start: Option<u64>,
    ) -> Value {
        json!({
            "timestamp": "2026-08-30T00:00:00Z",
            "ordinal": ordinal,
            "type": "session_meta",
            "payload": {
                "session_id": session,
                "id": thread,
                "parent_thread_id": parent,
                "timestamp": "2026-08-30T00:00:00Z",
                "cwd": "/workspace",
                "originator": "codex_cli_rs",
                "cli_version": "0.150.1",
                "history_mode": "paginated",
                "history_base": null,
                "subagent_history_start_ordinal": own_start,
                "agent_path": null
            }
        })
    }

    #[allow(
        clippy::needless_pass_by_value,
        reason = "the fixture builder intentionally consumes inline JSON values"
    )]
    fn event(ordinal: u64, payload: Value) -> Value {
        json!({
            "timestamp": "2026-08-30T00:00:00Z",
            "ordinal": ordinal,
            "type": "event_msg",
            "payload": payload
        })
    }

    #[allow(
        clippy::needless_pass_by_value,
        reason = "the fixture builder intentionally consumes inline JSON values"
    )]
    fn response(ordinal: u64, payload: Value) -> Value {
        json!({
            "timestamp": "2026-08-30T00:00:00Z",
            "ordinal": ordinal,
            "type": "response_item",
            "payload": payload
        })
    }

    fn usage(ordinal: u64, input: u64, cached: u64, output: u64) -> Value {
        event(
            ordinal,
            json!({
                "type": "token_count",
                "info": {
                    "total_token_usage": {
                        "input_tokens": input,
                        "cached_input_tokens": cached,
                        "cache_write_input_tokens": 0,
                        "output_tokens": output,
                        "reasoning_output_tokens": 0,
                        "total_tokens": input + output
                    },
                    "last_token_usage": {
                        "input_tokens": input,
                        "cached_input_tokens": cached,
                        "cache_write_input_tokens": 0,
                        "output_tokens": output,
                        "reasoning_output_tokens": 0,
                        "total_tokens": input + output
                    },
                    "model_context_window": 1000
                },
                "rate_limits": null
            }),
        )
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "the fixture verifies inherited-prefix exclusion across a complete parent-child Session"
    )]
    fn aggregates_threads_without_recounting_inherited_prefix() {
        let temp = TempDir::new().expect("temporary directory should be created");
        let root = temp.path().join("root.jsonl");
        let child = temp.path().join("child.jsonl");
        write_jsonl(
            &root,
            &[
                header(0, "session", "root", None, None),
                event(1, json!({"type":"task_started","turn_id":"root-turn"})),
                usage(2, 10, 8, 2),
                usage(3, 10, 8, 2),
                event(
                    4,
                    json!({
                        "type":"item_completed",
                        "item":{
                            "type":"SubAgentActivity",
                            "kind":"started",
                            "agent_thread_id":"child"
                        }
                    }),
                ),
                response(
                    5,
                    json!({"type":"custom_tool_call","call_id":"a","name":"exec","input":""}),
                ),
                event(
                    6,
                    json!({"type":"task_complete","turn_id":"root-turn","last_agent_message":null}),
                ),
            ],
            true,
        );
        write_jsonl(
            &child,
            &[
                header(0, "session", "child", Some("root"), Some(4)),
                header(1, "session", "root", None, None),
                event(2, json!({"type":"task_started","turn_id":"inherited"})),
                response(
                    3,
                    json!({"type":"custom_tool_call","call_id":"inherited","name":"exec","input":""}),
                ),
                event(4, json!({"type":"task_started","turn_id":"child-turn"})),
                response(
                    5,
                    json!({"type":"function_call","call_id":"b","name":"followup_task","arguments":"{}"}),
                ),
                usage(6, 5, 0, 1),
                event(
                    7,
                    json!({"type":"turn_aborted","turn_id":"child-turn","reason":"interrupted"}),
                ),
            ],
            true,
        );

        let report = inspect(&[root, child]).expect("valid session should be inspected");
        assert!(report.complete);
        assert_eq!(report.runtime_specific.facts["thread_count"], 2);
        assert_eq!(
            report
                .uniform
                .session
                .source_coverage
                .runtime_declared_child_sessions,
            1
        );
        assert!(
            report
                .uniform
                .session
                .source_coverage
                .missing_runtime_declared_child_sessions
                .is_empty()
        );
        assert_eq!(report.uniform.session.loops.total, 2);
        assert_eq!(report.uniform.session.loops.completed, 1);
        assert_eq!(report.uniform.session.loops.interrupted, 1);
        assert_eq!(report.uniform.session.items.agent_tool_calls.total, 2);
        assert_eq!(
            report.uniform.session.items.agent_tool_calls.by_name["exec"],
            1
        );
        assert_eq!(
            report.uniform.session.items.agent_tool_calls.by_name["followup_task"],
            1
        );
        let values = report
            .uniform
            .session
            .usage
            .values
            .expect("all threads reported usage");
        assert_eq!(values.input, 15);
        assert_eq!(values.cached, Some(8));
        assert_eq!(values.output, 3);
        assert_eq!(
            report.runtime_specific.facts["usage_accounting"]["reported_samples"],
            3
        );
        assert_eq!(
            report.runtime_specific.facts["usage_accounting"]["cumulative_updates"],
            2
        );
        assert_eq!(
            report.runtime_specific.facts["usage_accounting"]["duplicate_snapshots"],
            1
        );
        let included_records: u64 = report.runtime_specific.facts["sources"]
            .as_array()
            .expect("sources should be an array")
            .iter()
            .map(|source| source["included_records"].as_u64().expect("record count"))
            .sum();
        assert_eq!(included_records, 11);
    }

    #[test]
    fn rejects_mixed_native_sessions() {
        let temp = TempDir::new().expect("temporary directory should be created");
        let first = temp.path().join("first.jsonl");
        let second = temp.path().join("second.jsonl");
        write_jsonl(&first, &[header(0, "one", "one", None, None)], true);
        write_jsonl(&second, &[header(0, "two", "two", None, None)], true);
        let error = inspect(&[first, second]).expect_err("mixed sessions must fail");
        assert!(
            error
                .to_string()
                .contains("different native Codex sessions")
        );
    }

    #[test]
    fn rejects_history_base_until_lineage_is_supported() {
        let temp = TempDir::new().expect("temporary directory should be created");
        let path = temp.path().join("reverted.jsonl");
        let mut value = header(9, "session", "thread", None, None);
        value["payload"]["history_base"] = json!({
            "thread_id": "rollout",
            "end_ordinal_exclusive": 9,
            "end_byte_offset": 100
        });
        write_jsonl(&path, &[value], true);
        let error = inspect(&[path]).expect_err("history lineage must not be guessed");
        assert!(error.to_string().contains("history_base"));
    }

    #[test]
    fn sums_cumulative_segments_after_counter_reset() {
        let temp = TempDir::new().expect("temporary directory should be created");
        let path = temp.path().join("reset.jsonl");
        write_jsonl(
            &path,
            &[
                header(0, "session", "thread", None, None),
                usage(1, 10, 8, 2),
                usage(2, 5, 4, 1),
            ],
            true,
        );
        let report = inspect(&[path]).expect("a clean counter reset should start a new segment");
        let usage = report
            .uniform
            .session
            .usage
            .values
            .expect("the thread reported token usage");
        assert_eq!(usage.input, 15);
        assert_eq!(usage.output, 3);
        assert_eq!(usage.input + usage.output, 18);
        assert_eq!(
            report.runtime_specific.facts["usage_accounting"]["counter_resets"],
            1
        );
    }

    #[test]
    fn rejects_inconsistent_cumulative_fields_without_total_reset() {
        let temp = TempDir::new().expect("temporary directory should be created");
        let path = temp.path().join("inconsistent.jsonl");
        write_jsonl(
            &path,
            &[
                header(0, "session", "thread", None, None),
                usage(1, 10, 8, 2),
                usage(2, 11, 7, 2),
            ],
            true,
        );
        let error = inspect(&[path]).expect_err("mixed cumulative directions must fail");
        assert!(error.to_string().contains("fields are inconsistent"));
    }

    #[test]
    fn reports_valid_but_incomplete_sources() {
        let temp = TempDir::new().expect("temporary directory should be created");
        let path = temp.path().join("active.jsonl");
        write_jsonl(
            &path,
            &[
                header(0, "session", "thread", None, None),
                event(1, json!({"type":"task_started","turn_id":"open"})),
            ],
            false,
        );
        let report = inspect(&[path]).expect("a partial report is still inspectable");
        assert!(!report.complete);
        assert_eq!(report.uniform.session.loops.open, 1);
        assert!(
            report
                .incomplete_reasons
                .iter()
                .any(|reason| reason.starts_with("source_missing_terminal_newline:"))
        );
        assert!(
            report
                .incomplete_reasons
                .contains(&"token_usage_not_reported_by_every_thread".to_string())
        );
    }

    #[test]
    fn reports_a_missing_runtime_declared_child() {
        let temp = TempDir::new().expect("temporary directory should be created");
        let path = temp.path().join("root.jsonl");
        write_jsonl(
            &path,
            &[
                header(0, "session", "root", None, None),
                event(
                    1,
                    json!({
                        "type":"item_completed",
                        "item":{
                            "type":"SubAgentActivity",
                            "kind":"started",
                            "agent_thread_id":"missing-child"
                        }
                    }),
                ),
            ],
            true,
        );
        let report = inspect(&[path]).expect("missing child should produce a partial report");
        assert!(!report.complete);
        assert_eq!(report.uniform.session.source_coverage.status, "incomplete");
        assert_eq!(
            report
                .uniform
                .session
                .source_coverage
                .missing_runtime_declared_child_sessions,
            ["missing-child"]
        );
    }

    #[test]
    fn accepts_more_than_256_rollouts_in_one_native_session() {
        let temp = TempDir::new().expect("temporary directory should be created");
        let mut paths = Vec::new();
        for index in 0..257_u64 {
            let path = temp.path().join(format!("rollout-{index}.jsonl"));
            write_jsonl(
                &path,
                &[header(
                    0,
                    "large-session",
                    format!("thread-{index}").as_str(),
                    None,
                    None,
                )],
                true,
            );
            paths.push(path);
        }
        let report = inspect(&paths).expect("the documented source bound should accept 257 files");
        assert_eq!(report.uniform.session.source_count, 257);
        assert_eq!(report.runtime_specific.facts["thread_count"], 257);
    }
}
