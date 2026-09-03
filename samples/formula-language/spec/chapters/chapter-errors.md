+++
schema = "example/formula-language/chapter@1"
id = "chapter-errors"
title = "Parse errors"
sequence = 5
status = "draft"
+++

# Parse errors

## Purpose

The parser stops at the first error and reports its kind and byte offset.
Formulo does not attempt recovery. Every error kind is exercised by at least
one example in the conformance corpus.
