+++
schema = "example/formula-language/example@1"
id = "example-decimal"
source = "=1.50 + .5"
expected_error = "error-invalid-character"
exercises = ["production-additive"]
+++

# Example `decimal`

A literal needs a leading digit. `.5` starts no token, so the lexer stops
at `.` with an invalid character.
