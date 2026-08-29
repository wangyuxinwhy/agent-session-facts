use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
pub(crate) struct Report {
    pub(crate) schema_version: u32,
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
    pub(crate) runtime: &'static str,
    pub(crate) complete: bool,
    pub(crate) incomplete_reasons: Vec<String>,
    pub(crate) uniform: UniformFacts,
    pub(crate) runtime_specific: RuntimeSpecificFacts,
}

#[derive(Debug, Serialize)]
pub(crate) struct UniformFacts {
    pub(crate) schema: &'static str,
    pub(crate) session: SessionFacts,
    pub(crate) sources: Vec<SourceFacts>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RuntimeSpecificFacts {
    pub(crate) schema: &'static str,
    pub(crate) facts: Value,
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionFacts {
    pub(crate) native_id: String,
    pub(crate) source_count: u64,
    pub(crate) runtime_versions: Vec<String>,
    pub(crate) source_coverage: SourceCoverageFacts,
    pub(crate) loops: LoopFacts,
    pub(crate) usage: UsageFacts,
    pub(crate) items: ItemFacts,
    pub(crate) model_cost: ModelCostFacts,
}

#[derive(Debug, Serialize)]
pub(crate) struct SourceCoverageFacts {
    pub(crate) status: &'static str,
    pub(crate) basis: &'static str,
    pub(crate) supplied_sources: u64,
    pub(crate) runtime_declared_child_sessions: u64,
    pub(crate) missing_runtime_declared_child_sessions: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct LoopFacts {
    pub(crate) total: u64,
    pub(crate) completed: u64,
    pub(crate) failed: u64,
    pub(crate) interrupted: u64,
    pub(crate) ended_without_outcome: u64,
    pub(crate) open: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Usage {
    pub(crate) input: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cached: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cache_write: Option<u64>,
    pub(crate) output: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning: Option<u64>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AvailabilityStatus {
    Available,
    Partial,
    Unavailable,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct UsageFacts {
    pub(crate) status: AvailabilityStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) values: Option<Usage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) known_values: Option<Usage>,
}

#[derive(Debug, Default, Serialize)]
pub(crate) struct ItemFacts {
    #[serde(rename = "agent.tool_call")]
    pub(crate) agent_tool_calls: ToolCallFacts,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct ToolCallFacts {
    pub(crate) total: u64,
    pub(crate) by_name: BTreeMap<String, u64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ModelCostFacts {
    pub(crate) status: AvailabilityStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) amount_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) basis: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SourceFacts {
    pub(crate) path: String,
    pub(crate) sha256: String,
    pub(crate) bytes: u64,
    pub(crate) records: u64,
    pub(crate) terminal_newline: bool,
}
