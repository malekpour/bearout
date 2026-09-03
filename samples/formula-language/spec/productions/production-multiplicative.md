+++
schema = "example/formula-language/production@1"
id = "production-multiplicative"
rule = "production-power { ( token-star | token-slash ) production-power }"
uses = ["production-power", "token-star", "token-slash"]
chapter = "chapter-expressions"

+++

# Production `multiplicative`

Multiplication and division. Left associative.
