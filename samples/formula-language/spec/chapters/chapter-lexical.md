+++
schema = "example/formula-language/chapter@1"
id = "chapter-lexical"
title = "Lexical structure"
sequence = 1
status = "draft"
+++

# Lexical structure

## Purpose

How source text becomes tokens. A formula begins with `=`. Whitespace (spaces
and tabs) separates tokens and is otherwise ignored. The decimal separator is
`.` and the argument separator is `,`. Function names and the keywords `TRUE`
and `FALSE` are case-insensitive and normalized to upper case; cell column
letters are normalized to upper case as well. Numeric literals keep their
spelling verbatim in the AST: Formulo assigns no numeric value to them.
