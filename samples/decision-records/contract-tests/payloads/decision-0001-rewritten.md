+++
schema = "example/decision-records/decision@1"
id = "decision-0001"
title = "Decision records get citable rulings"
status = "accepted"
date = "2026-08-20"

+++

# Decision records get citable rulings

## Context

A decision that cannot be cited cannot be relied on. A reader who follows
[decision-0001-ruling-01](#decision-0001-ruling-01) must find one sentence
with one meaning.

## Decision

Every accepted record publishes its rulings as fragments under headings
named by the ruling identifier.

## Rulings

### decision-0001-ruling-01

```toml bearout=ruling
id = "decision-0001-ruling-01"
text = "Rewritten."
```

### decision-0001-ruling-02

```toml bearout=ruling
id = "decision-0001-ruling-02"
text = "Every ruling carries a stable identifier derived from its record identifier."
```
