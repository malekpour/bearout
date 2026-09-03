+++
schema = "example/formula-language/production@1"
id = "production-unary"
rule = "( token-plus | token-minus ) production-unary | production-primary"
uses = ["token-plus", "token-minus", "production-unary", "production-primary"]
chapter = "chapter-expressions"

+++

# Production `unary`

Unary plus and minus, binding tighter than every binary operator.
