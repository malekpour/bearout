# SPDX-License-Identifier: Apache-2.0
# Entry module. Registers the decision schema, the log-wide invariants, and
# the index generator. The kernel checks the shape before any of these run.

load("decision.star", "validate_decision")
load("log.star", "check_numbering_is_contiguous", "check_supersession_is_reciprocal")
load("immutability.star", "check_protected_records")
load("decision-index.star", "plan_decision_index")

schema(
    "example/decision-records/decision@1",
    shape = "decision.schema.toml",
    validate = validate_decision,
)
check("supersession-is-reciprocal", check_supersession_is_reciprocal)
check("numbering-is-contiguous", check_numbering_is_contiguous)
check("protected-records-are-immutable", check_protected_records)
generator("decision-index", plan_decision_index)
