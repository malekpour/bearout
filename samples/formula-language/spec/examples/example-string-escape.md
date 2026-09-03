+++
schema = "example/formula-language/example@1"
id = "example-string-escape"
source = '="say ""hi"""'
expected_ast = '(string "say \"hi\"")'
exercises = ["production-primary"]
+++

# Example `string-escape`

Doubled quotes inside a string stand for one quote.
