+++
schema = "example/decision-records/decision@1"
id = "decision-0004"
title = "Records are numbered when merged"
status = "accepted"
date = "2026-09-02"
supersedes = ["decision-0002"]
+++

# Records are numbered when merged

## Context

[decision-0002-ruling-01](decision-0002.md#decision-0002-ruling-01)
allocated numbers at drafting time, which produced collisions between
concurrent branches.

## Decision

Numbers are allocated on merge. This supersedes
[decision-0002](decision-0002.md).

## Rulings

### decision-0004-ruling-01

```toml bearout=ruling
id = "decision-0004-ruling-01"
text = "A record receives its sequence number when it is merged, never when it is drafted."
```

### decision-0004-ruling-02

```toml bearout=ruling
id = "decision-0004-ruling-02"
text = "Drafts use a provisional `decision-xxxx` identifier that the merge replaces."
```
