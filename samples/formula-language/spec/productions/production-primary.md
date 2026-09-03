+++
schema = "example/formula-language/production@1"
id = "production-primary"
rule = "token-number | token-string | token-true | token-false | production-reference | production-call | token-lparen production-comparison token-rparen"
uses = ["token-number", "token-string", "token-true", "token-false", "production-reference", "production-call", "token-lparen", "production-comparison", "token-rparen"]
chapter = "chapter-values"

+++

# Production `primary`

A literal, a reference, a call, or a parenthesised expression.
