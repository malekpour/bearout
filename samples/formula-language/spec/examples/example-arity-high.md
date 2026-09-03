+++
schema = "example/formula-language/example@1"
id = "example-arity-high"
source = "=IF(1, 2, 3, 4)"
expected_error = "error-wrong-argument-count"
exercises = ["production-call"]
functions = ["function-if"]
+++

# Example `arity-high`

`IF` accepts at most three arguments.
