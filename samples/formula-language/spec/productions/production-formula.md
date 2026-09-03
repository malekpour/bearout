+++
schema = "example/formula-language/production@1"
id = "production-formula"
rule = "token-equal production-comparison"
uses = ["token-equal", "production-comparison"]
chapter = "chapter-lexical"
start = true
+++

# Production `formula`

A formula is a leading `=` followed by one expression. This is the start
production.
