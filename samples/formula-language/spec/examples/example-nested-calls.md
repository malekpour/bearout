+++
schema = "example/formula-language/example@1"
id = "example-nested-calls"
source = "=IF(A1 > 0, MAX(A1, 1), MIN(A1, -1))"
expected_ast = "(call IF (binary greater (cell A1) (number 0)) (call MAX (cell A1) (number 1)) (call MIN (cell A1) (unary negate (number 1))))"
exercises = ["production-call"]
functions = ["function-if", "function-max", "function-min"]
+++

# Example `nested-calls`

Calls nest and take expressions as arguments.
