+++
schema = "example/formula-language/example@1"
id = "example-cells"
source = "=$a$1 + b2"
expected_ast = "(binary add (cell $A$1) (cell B2))"
exercises = ["production-reference"]

+++

# Example `cells`

Column letters are normalized to upper case; `$` marks absolute parts.
