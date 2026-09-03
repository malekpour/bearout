+++
schema = "example/formula-language/token@1"
id = "token-cell"
kind = "reference"
variant = "Cell"
pattern = "\\$?[A-Za-z]+\\$?[0-9]+"
chapter = "chapter-values"
+++

# Token `cell`

A cell reference. Letters are normalized to upper case; `$` marks an
absolute column or row.
