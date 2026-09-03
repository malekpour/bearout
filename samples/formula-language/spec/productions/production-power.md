+++
schema = "example/formula-language/production@1"
id = "production-power"
rule = "production-unary [ token-caret production-power ]"
uses = ["production-unary", "token-caret", "production-power"]
chapter = "chapter-expressions"

+++

# Production `power`

Exponentiation. Right associative, and its operands are unary expressions,
so `=-2^2` parses as `(-2)^2`.
