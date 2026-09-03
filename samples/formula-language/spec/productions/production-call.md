+++
schema = "example/formula-language/production@1"
id = "production-call"
rule = "token-identifier token-lparen [ production-comparison { token-comma production-comparison } ] token-rparen"
uses = ["token-identifier", "token-lparen", "production-comparison", "token-comma", "token-rparen"]
chapter = "chapter-functions"

+++

# Production `call`

A function call. The name must be in the registry and the argument count
within its arity.
