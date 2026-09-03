+++
schema = "example/formula-language/example@1"
id = "example-comparison-chain"
source = "=1 < 2 <> 3"
expected_ast = "(binary not-equal (binary less (number 1) (number 2)) (number 3))"
exercises = ["production-comparison"]

+++

# Example `comparison-chain`

Comparisons are left associative.
