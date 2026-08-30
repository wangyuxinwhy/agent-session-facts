# agent-session-facts

`agent-session-facts` extracts auditable execution facts directly from native Codex, Pi, and Claude Code Session JSONL artifacts. It is an independent read-only binary: it does not call RunLab or Trace Index and does not judge task quality.

Documentation: <https://wangyuxinwhy.github.io/agent-session-facts/>

## Installation

```bash
cargo install agent-session-facts --version 0.1.0 --locked
```

Prebuilt Linux and macOS archives are also attached to the matching [GitHub Release](https://github.com/wangyuxinwhy/agent-session-facts/releases).

```bash
agent-session-facts session inspect --runtime codex root.jsonl child.jsonl
agent-session-facts session inspect --runtime pi session.jsonl
agent-session-facts session inspect --runtime claude session.jsonl subagents/*.jsonl
```

Stdout contains one compact `agent_session_facts.report` JSON object. Diagnostics use one compact `agent_session_facts.error` JSON object on stderr. Exit status 2 denotes invalid or unsupported input, 1 denotes I/O or internal failure, and 0 denotes a produced report. A readable but incomplete artifact set is represented by `complete: false`, not a process error.

## Two views, one report

Every runtime emits the same envelope and the same `uniform` shape:

```json
{
  "schema_version": 1,
  "type": "agent_session_facts.report",
  "runtime": "codex",
  "complete": true,
  "incomplete_reasons": [],
  "uniform": {
    "schema": "agent_session_facts.uniform@1",
    "session": {
      "native_id": "...",
      "source_count": 1,
      "runtime_versions": [],
      "source_coverage": {},
      "loops": {},
      "usage": {},
      "items": {"agent.tool_call": {}},
      "model_cost": {}
    },
    "sources": []
  },
  "runtime_specific": {
    "schema": "agent_session_facts.codex@1",
    "facts": {}
  }
}
```

The uniform terminology follows the useful part of Trace Index's cross-runtime mental model: a runtime-maintained `session` contains outer execution `loops`, relevant program `items`, and is supported by physical `sources`. This tool does not reproduce Trace Index's Record, Blob, semantic-text, indexing, or search contracts because the Session accounting task does not need them.

`uniform.session.usage` uses the shared normalized shape:

```text
input        all model input, including cache reads and writes
cached       optional subset of input
cache_write  optional subset of input
output       all model output, including reasoning
reasoning    optional subset of output
```

Missing optional usage is `null`, meaning unknown rather than zero. Uniform fields are always present so consumers see the same JSON shape for every runtime. Total model tokens are derived as `input + output`; no redundant total is stored. `available`, `partial`, and `unavailable` distinguish complete values from known lower-bound values and missing evidence.

Only facts with the same meaning across all three runtimes enter `uniform`. Runtime-native topology, checkpoint behavior, format versions, and native event kinds remain under the versioned `runtime_specific` view:

- `agent_session_facts.codex@1` preserves Thread lineage, inherited-history ordinals, native tool kinds, and cumulative usage checkpoint deduplication/reset facts.
- `agent_session_facts.pi@1` preserves the entry tree, Session format version, native stop reasons, one-hour cache writes, and per-response usage/cost accounting.
- `agent_session_facts.claude@1` preserves root/subagent contexts, the UUID record tree, native tool kinds, and assistant response snapshot deduplication facts.

## Runtime input and accounting

### Codex

Supply every on-disk rollout JSONL belonging to one native `session_id`. A Codex Session can contain several Thread rollout files. Child rollouts contain an inherited parent-history prefix; the adapter excludes records before `subagent_history_start_ordinal` before aggregating facts.

Repeated `token_count` events may carry the same cumulative `total_token_usage`. The adapter does not sum `last_token_usage` samples. It deduplicates cumulative snapshots and sums monotonic counter segments when an older runtime resets a Thread counter. Codex rollout JSONL does not record model cost, so cost is explicitly unavailable.

The adapter rejects the distinct `codex exec --json` stream, mixed native Session IDs, duplicate rollout inputs, unsafe child-history boundaries, and `history_base` revert lineage until a representative real artifact establishes exact accounting.

### Pi

Supply exactly one native Pi Session JSONL. Resume appends to that file; `parentSession` denotes a separately identified fork. Usage and cost are summed over assistant, tool-result, compaction, and branch-summary records using Pi's upstream `getSessionStats` accounting semantics. Normalized input adds native `input + cacheRead + cacheWrite`.

Pi's recorded cost is published with basis `runtime_recorded`. It is a fact from the runtime artifact, not an independently verified provider invoice.

### Claude Code

Supply exactly one root Session JSONL and every runtime-declared subagent JSONL for complete coverage. All sources must carry one native `sessionId`; each subagent source must carry a unique `agentId`.

Claude Code can split one API response across several assistant records. Those records repeat or progressively update the same full usage object under one `message.id`. The adapter keeps the latest monotonic snapshot per message identity before summing, and exposes physical, duplicate, and progressive snapshot counts for audit. Tool calls are deduplicated by native tool-use identity. Claude Session JSONL does not record model cost, so cost is explicitly unavailable.

## Evidence and completeness

Every source reports its canonical path, byte size, SHA-256 digest, record count, and terminal-newline state. The digest fixes the inspected snapshot.

`complete: true` means the supplied source set satisfies the runtime-specific coverage rules, the files remained stable and structurally complete while read, every observed Loop has an ending, and the adapter found enough native evidence for complete accounting. `complete: false` retains all supported known facts and names every detected gap in `incomplete_reasons`.
