+++
schema = "example/formula-language/example@1"
id = "example-precedence"
source = "=1 + 2 * 3"
expected_ast = "(binary add (number 1) (binary multiply (number 2) (number 3)))"
exercises = ["production-additive", "production-multiplicative"]

+++

# Example `precedence`

Multiplication binds tighter than addition.
