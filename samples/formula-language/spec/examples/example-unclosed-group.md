+++
schema = "example/formula-language/example@1"
id = "example-unclosed-group"
source = "=(1 + 2"
expected_error = "error-unexpected-end"
exercises = ["production-primary"]

+++

# Example `unclosed-group`

The group never closes.
