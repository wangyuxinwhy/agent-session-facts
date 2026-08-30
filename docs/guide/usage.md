---
description: Inspect Codex, Pi, and Claude Code Session artifacts with agent-session-facts.
---

# Usage

The command reads explicit JSONL artifacts and writes one compact JSON report to stdout:

```bash
agent-session-facts session inspect --runtime <codex|pi|claude> ARTIFACT...
```

Diagnostics are one compact JSON error on stderr. Exit status `0` means a report was produced, including a report with `complete: false`; status `1` is an I/O or internal failure, and status `2` is invalid or unsupported input.

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
