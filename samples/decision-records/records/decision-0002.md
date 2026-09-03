+++
schema = "example/decision-records/decision@1"
id = "decision-0002"
title = "Records are numbered when drafted"
status = "superseded"
date = "2026-08-21"
superseded_by = "decision-0004"
+++

# Records are numbered when drafted

## Context

Sequential numbers are easy to allocate but collide when two branches add a
record at the same time.

## Decision

Allocate `decision-NNNN` when the draft is opened. Superseded by
[decision-0004](decision-0004.md).

## Rulings

### decision-0002-ruling-01

```toml bearout=ruling
id = "decision-0002-ruling-01"
text = "Record identifiers are `decision-` followed by a zero-padded sequence number."
```
