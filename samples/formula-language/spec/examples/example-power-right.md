+++
schema = "example/formula-language/example@1"
id = "example-power-right"
source = "=2^3^2"
expected_ast = "(binary power (number 2) (binary power (number 3) (number 2)))"
exercises = ["production-power"]

+++

# Example `power-right`

Exponentiation is right associative.
