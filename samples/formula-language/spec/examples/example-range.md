+++
schema = "example/formula-language/example@1"
id = "example-range"
source = "=SUM(A1:B3)"
expected_ast = "(call SUM (range A1 B3))"
exercises = ["production-call", "production-reference"]
functions = ["function-sum"]
+++

# Example `range`

A range argument.
