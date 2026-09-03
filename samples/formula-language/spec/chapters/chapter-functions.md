+++
schema = "example/formula-language/chapter@1"
id = "chapter-functions"
title = "Function calls"
sequence = 4
status = "draft"
+++

# Function calls

## Purpose

A call is an identifier followed by a parenthesised, comma-separated
argument list. The registry of known functions and their arities is part of
the language definition; an unknown name or a wrong argument count is a parse
error, not a runtime error.
