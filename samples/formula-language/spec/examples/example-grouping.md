+++
schema = "example/formula-language/example@1"
id = "example-grouping"
source = "=(1 + 2) * 3"
expected_ast = "(binary multiply (binary add (number 1) (number 2)) (number 3))"
exercises = ["production-primary"]

+++

# Example `grouping`

Parentheses override precedence.
