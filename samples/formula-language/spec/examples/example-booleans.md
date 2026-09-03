+++
schema = "example/formula-language/example@1"
id = "example-booleans"
source = "=true = FALSE"
expected_ast = "(binary equal (bool true) (bool false))"
exercises = ["production-comparison"]

+++

# Example `booleans`

Keywords are case-insensitive; the second `=` is the comparison operator.
