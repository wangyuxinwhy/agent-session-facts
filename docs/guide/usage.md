---
description: Inspect Codex, Pi, and Claude Code Session artifacts with agent-session-facts.
---

# Usage

Choose the runtime adapter, then supply the native JSONL files that belong to one logical Session:

```bash
agent-session-facts session inspect --runtime <codex|pi|claude> ARTIFACT...
```

The command reconstructs the Session, normalizes shared facts, preserves runtime-native details, and writes one compact JSON report to stdout. Save it directly when another program will consume it:

```bash
agent-session-facts session inspect --runtime pi session.jsonl > report.json
```

Diagnostics are one compact JSON error on stderr. Exit status `0` means a report was produced, including a report with `complete: false`; status `1` is an I/O or internal failure, and status `2` is invalid or unsupported input.

## Reading the result

Start with these fields:

- `complete` and `incomplete_reasons`: whether the supplied artifacts support a complete report.
- `uniform.session.loops`: execution-cycle totals and outcomes.
- `uniform.session.usage`: normalized model-token usage.
- `uniform.session.items["agent.tool_call"]`: total tool calls and counts by tool name.
- `uniform.session.model_cost`: runtime-recorded cost or the reason it is unavailable.
- `uniform.sources`: the exact physical evidence inspected.
- `runtime_specific`: native accounting and topology details for the selected runtime.

### Cache reads and writes

Inside `uniform.session.usage`, `cached` and `cache_write` describe opposite cache directions:

- `cached`: input tokens served from an existing prompt cache during this request.
- `cache_write`: input tokens newly stored in the prompt cache for later requests.

Both values are already included in `input`. If a report says `input: 1000`, `cached: 600`, and `cache_write: 200`, the total input is 1000—not 1800. The remaining 200 input tokens were neither read from nor written to cache.

## Codex

Supply every on-disk rollout belonging to one native `session_id`:

```bash
agent-session-facts session inspect \
  --runtime codex \
  root-rollout.jsonl \
  child-rollout.jsonl
```

Child Thread rollouts repeat inherited parent history. The adapter excludes records before `subagent_history_start_ordinal`, deduplicates cumulative usage snapshots, and reports unsupported `history_base` lineage instead of guessing. The separate `codex exec --json` stream is not accepted.

## Pi

Supply exactly one native Pi Session JSONL:

```bash
agent-session-facts session inspect --runtime pi session.jsonl
```

Pi resumes append to the same file. A `parentSession` is a separately identified fork. Usage and runtime-recorded cost are summed across assistant, tool-result, compaction, and branch-summary entries.

## Claude Code

Supply one root source and every runtime-declared subagent source with the same `sessionId`:

```bash
agent-session-facts session inspect \
  --runtime claude \
  session.jsonl \
  subagents/*.jsonl
```

Claude Code can split one API response into several assistant records. Usage is deduplicated by `message.id`, keeping the latest monotonic snapshot; tool calls are deduplicated by native tool-use identity.

## Completeness

Always inspect both fields:

```json
{
  "complete": false,
  "incomplete_reasons": ["open_loops"]
}
```

An incomplete report retains every supported known fact. Missing values are never silently converted to zero or estimated from text.
