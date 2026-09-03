+++
schema = "example/engineering-evidence/decision@1"
id = "decision-0001"
title = "The core is the single backplane controller"
status = "accepted"
basis = "analysis"
date = "2026-08-25"
closes = ["question-0003"]
+++

# The core is the single backplane controller

## Context

A deterministic cyclic bound is a property of the access discipline, not of
the wire. Settling who may transmit lets architecture proceed before any
physical layer is exercised. This decision rests on analysis and cites no
measurement; it closes [question-0003](../questions/question-0003.md).

## Decision

Core-controlled access with distinct service classes for cyclic and bulk
traffic.

## Rulings

### decision-0001-ruling-01

```toml bearout=ruling
id = "decision-0001-ruling-01"
text = "Exactly one core is the bus controller and primary timebase of one backplane."
```

### decision-0001-ruling-02

```toml bearout=ruling
id = "decision-0001-ruling-02"
text = "Modules do not initiate transmissions; the core grants every opportunity."
```
