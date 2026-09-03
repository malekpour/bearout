# SPDX-License-Identifier: Apache-2.0
# Entry module. Every value in this sample is a synthetic fixture; the
# schemas require each one to say so and to trace to a source.

load("interface.star", "validate_interface")
load("question.star", "validate_question")
load("decision.star", "validate_decision")
load("register.star", "check_blocked_only_by_open_questions", "check_closure_is_reciprocal", "check_measurement_basis_cites_measurements")
load("registers.star", "plan_registers")

NS = "example/engineering-evidence/"

schema(NS + "question@1", shape = "question.schema.toml", validate = validate_question)
schema(NS + "decision@1", shape = "decision.schema.toml", validate = validate_decision)
schema(NS + "source@1", shape = "source.schema.toml")
schema(NS + "measurement@1", shape = "measurement.schema.toml")
schema(NS + "interface@1", shape = "interface.schema.toml", validate = validate_interface)
schema(NS + "module@1", shape = "module.schema.toml")

check("closure-is-reciprocal", check_closure_is_reciprocal)
check("blocked-only-by-open-questions", check_blocked_only_by_open_questions)
check("measurement-basis-cites-measurements", check_measurement_basis_cites_measurements)

generator("question-register", plan_registers)
