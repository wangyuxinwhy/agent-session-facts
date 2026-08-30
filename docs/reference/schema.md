---
description: Versioned JSON output schema shared by Codex, Pi, and Claude Code Session reports.
---

# Output Schema

Every successful inspection emits one `agent_session_facts.report` JSON object. The report has two independently versioned views:

- `uniform` contains facts with the same meaning across Codex, Pi, and Claude Code.
- `runtime_specific` preserves native topology and accounting details that cannot be normalized without losing meaning.

## Envelope

```json
{
  "schema_version": 1,
  "type": "agent_session_facts.report",
  "runtime": "codex",
  "complete": true,
  "incomplete_reasons": [],
  "uniform": {
    "schema": "agent_session_facts.uniform@1",
    "session": {},
    "sources": []
  },
  "runtime_specific": {
    "schema": "agent_session_facts.codex@1",
    "facts": {}
  }
}
```

`runtime` is one of `codex`, `pi`, or `claude`. The runtime-specific schema identifier is correspondingly `agent_session_facts.codex@1`, `agent_session_facts.pi@1`, or `agent_session_facts.claude@1`.

## Uniform Session

```json
{
  "native_id": "runtime-owned-session-id",
  "source_count": 1,
  "runtime_versions": ["2.1.232"],
  "source_coverage": {
    "status": "complete",
    "basis": "single_native_pi_session_source",
    "supplied_sources": 1,
    "runtime_declared_child_sessions": 0,
    "missing_runtime_declared_child_sessions": []
  },
  "loops": {
    "total": 2,
    "completed": 1,
    "failed": 0,
    "interrupted": 1,
    "ended_without_outcome": 0,
    "open": 0
  },
  "usage": {
    "status": "available",
    "values": {
      "input": 801795,
      "cached": 706624,
      "cache_write": 0,
      "output": 11186,
      "reasoning": 5476
    },
    "known_values": null
  },
  "items": {
    "agent.tool_call": {
      "total": 29,
      "by_name": {
        "bash": 17,
        "read": 8
      }
    }
  },
  "model_cost": {
    "status": "available",
    "amount_usd": 0.6652902,
    "basis": "runtime_recorded",
    "reason": null
  }
}
```

### Loop outcomes

| Field | Meaning |
|---|---|
| `completed` | The runtime stated successful completion. |
| `failed` | The runtime stated failure. |
| `interrupted` | The runtime stated interruption or abort. |
| `ended_without_outcome` | An ending boundary exists, but the runtime did not state an outcome. |
| `open` | No ending boundary was observed. |

The outcome counts sum to `total`.

### Usage

Normalized usage follows the same subset semantics for all runtimes:

| Field | Meaning |
|---|---|
| `input` | All model input, including cache reads and writes. |
| `cached` | Optional subset of `input` read from cache. |
| `cache_write` | Optional subset of `input` written to cache. |
| `output` | All model output, including reasoning. |
| `reasoning` | Optional subset of `output`. |

Total model tokens are derived as `input + output`; no redundant total is stored. Every uniform field is always serialized. Optional values use `null` when unknown; `null` is not zero.

`status` is:

- `available`: complete values are in `values`; `known_values` is `null`.
- `partial`: safely known values are in `known_values`; `values` is `null`.
- `unavailable`: both value fields are `null`.

### Model cost

`model_cost` uses the same availability status. Pi can publish `amount_usd` with basis `runtime_recorded`; this preserves what Pi wrote and is not an independently verified provider invoice. Codex and Claude Code currently report cost as unavailable with a `reason`.

## Sources

Each source fixes the inspected physical snapshot:

```json
{
  "path": "/canonical/path/to/session.jsonl",
  "sha256": "0123456789abcdef...",
  "bytes": 12345,
  "records": 42,
  "terminal_newline": true
}
```

## Runtime-specific facts

Runtime-specific fields are intentionally not forced into one common body:

| Schema | Preserved facts |
|---|---|
| `agent_session_facts.codex@1` | Thread lineage, inherited-history ordinals, native tool kinds, cumulative usage samples, duplicate snapshots, and counter resets. |
| `agent_session_facts.pi@1` | Entry tree, Session format version, message roles, stop reasons, one-hour cache writes, known reasoning, and runtime-recorded cost coverage. |
| `agent_session_facts.claude@1` | Root/subagent contexts, UUID record trees, native tool kinds, and physical/progressive/duplicate assistant usage snapshots. |

Follow the `runtime_specific.schema` identifier before interpreting `facts`. New incompatible uniform meanings require `agent_session_facts.uniform@2`; incompatible CLI contracts require an appropriate crate version change.
