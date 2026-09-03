+++
schema = "example/formula-language/example@1"
id = "example-associativity"
source = "=8 - 3 - 2"
expected_ast = "(binary subtract (binary subtract (number 8) (number 3)) (number 2))"
exercises = ["production-additive"]

+++

# Example `associativity`

Subtraction is left associative.
