+++
schema = "example/formula-language/operator@1"
id = "operator-power"
name = "power"
symbol = "^"
arity = "binary"
precedence = 4
associativity = "right"
token = "token-caret"
production = "production-power"
+++

# Operator `^`

Exponentiation. Right associative: `=2^3^2` is `2^(3^2)`.
