+++
schema = "example/formula-language/example@1"
id = "example-unary-before-power"
source = "=-2^2"
expected_ast = "(binary power (unary negate (number 2)) (number 2))"
exercises = ["production-unary", "production-power"]

+++

# Example `unary-before-power`

Unary minus binds tighter than exponentiation.
