---
pageType: home
hero:
  name: agent-session-facts
  text: Understand an Agent Session from its native logs
  tagline: Turn Codex, Pi, and Claude Code JSONL artifacts into one auditable report of loops, model usage, tool calls, cost, and source coverage.
  actions:
    - text: Get Started
      link: /guide/overview
      theme: brand
    - text: View Schema
      link: /reference/schema
      theme: alt
features:
  - title: Reconstruct the whole Session
    details: Combine root, child, rollout, and subagent files without double-counting inherited history or repeated usage snapshots.
    icon: 🧩
  - title: Compare the same facts
    details: Read one stable Session, Loop, usage, tool-call, cost, and Source shape across three runtime formats.
    icon: ↔️
  - title: Audit every result
    details: Keep coverage, incomplete reasons, canonical paths, byte counts, record counts, and SHA-256 digests with the report.
    icon: 🔎
---

## What it answers

Given the native files for one Agent Session, `agent-session-facts` answers:

- How many outer execution loops completed, failed, were interrupted, or remain open?
- How much input was processed, how much came from an existing cache, and how much was written for future requests?
- Which tools were called, and how often?
- Did the runtime record model cost?
- Were all runtime-declared child or subagent sources supplied?
- Which exact files and records support the result?

The result is one compact JSON object. Shared facts have the same fields for every supported runtime; native details that cannot be normalized safely remain in a versioned runtime-specific section.

## Typical workflow

```bash
agent-session-facts session inspect \
  --runtime claude \
  session.jsonl \
  subagents/*.jsonl > report.json
```

Start with the [overview](/guide/overview), then see [installation](/guide/installation), [usage](/guide/usage), and the complete [output schema](/reference/schema).
