+++
schema = "example/formula-language/example@1"
id = "example-whitespace"
source = "=  1+\t2 "
expected_ast = "(binary add (number 1) (number 2))"
exercises = ["production-additive"]

+++

# Example `whitespace`

Spaces and tabs between tokens are ignored, including at the end.
