+++
schema = "example/formula-language/production@1"
id = "production-additive"
rule = "production-multiplicative { ( token-plus | token-minus ) production-multiplicative }"
uses = ["production-multiplicative", "token-plus", "token-minus"]
chapter = "chapter-expressions"

+++

# Production `additive`

Addition and subtraction. Left associative.
