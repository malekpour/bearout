+++
schema = "example/formula-language/production@1"
id = "production-comparison"
rule = "production-additive { ( token-equal | token-not-equal | token-less | token-less-equal | token-greater | token-greater-equal ) production-additive }"
uses = ["production-additive", "token-equal", "token-not-equal", "token-less", "token-less-equal", "token-greater", "token-greater-equal"]
chapter = "chapter-expressions"

+++

# Production `comparison`

Comparison, the loosest binding level. Left associative, so `=1<2<3`
parses as `(1<2)<3`.
