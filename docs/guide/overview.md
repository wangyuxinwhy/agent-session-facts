---
description: What agent-session-facts reads, what it reports, and when to use it.
---

# Overview

Agent runtimes persist useful execution facts, but each runtime uses a different artifact format. A single logical Session may also span several rollout, child, or subagent files. Reading one file at a time can miss part of the Session or count inherited history and repeated usage snapshots more than once.

`agent-session-facts` provides one command for that problem. Give it the native JSONL artifacts from one Codex, Pi, or Claude Code Session, and it emits one compact JSON report.

## What the report contains

The shared `uniform` view contains facts with the same meaning across supported runtimes:

- Session identity, runtime versions, and source coverage.
- Loop totals and outcomes.
- Normalized model-token usage.
- Tool-call totals and counts by tool name.
- Model cost when the runtime recorded it.
- Exact source paths, byte sizes, record counts, terminal-newline state, and SHA-256 digests.

The `runtime_specific` view retains native details that would lose meaning if forced into the shared shape, such as Codex Thread lineage, Pi entry-tree accounting, or Claude response-snapshot deduplication.

## When to use it

Use `agent-session-facts` when you need to:

- Understand how one Agent Session executed.
- Aggregate a Session that spans several native files.
- Compare usage and tool activity across Codex, Pi, and Claude Code.
- Supply a stable machine-readable report to scripts, dashboards, or other automation.
- Audit a result back to the exact source artifacts.

The command reads only the files you explicitly provide. When evidence is missing, incomplete, or unsupported, the report preserves the known facts and marks the gap instead of inventing a value.

Continue with [installation](/guide/installation), then run your first [inspection](/guide/usage).
