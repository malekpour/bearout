+++
schema = "example/engineering-evidence/decision@1"
id = "decision-0002"
title = "Digital inputs switch at the fixture threshold"
status = "accepted"
basis = "measurement"
date = "2026-08-28"
closes = ["question-0004"]
evidence = ["measurement-input-threshold", "measurement-input-hysteresis"]
+++

# Digital inputs switch at the fixture threshold

## Context

This decision's basis is measurement, so the shape requires evidence. Both
cited records are synthetic fixture values from
[source-fixture-bench](../sources/source-fixture-bench.md); they stand in
for real bench data only to exercise the evidence relationships. It closes
[question-0004](../questions/question-0004.md).

## Decision

The digital input module declares the fixture threshold and hysteresis as
its figures.

## Rulings

### decision-0002-ruling-01

```toml bearout=ruling
id = "decision-0002-ruling-01"
text = "A module figure names a measurement record; free-text figures are not accepted."
```
