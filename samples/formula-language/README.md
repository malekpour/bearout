# Sample: formula-language

## Purpose

The definition of **Formulo**, a deliberately small spreadsheet-expression
language, as a resource graph, together with the Rust crate that Bearout
generates from it: an AST, a lexer, a parser, and a conformance test suite
that compile and pass in CI. Formulo is **not Excel-compatible** and **not
OpenFormula-conformant**. OpenFormula is prior art only:
<https://docs.oasis-open.org/office/OpenDocument/v1.3/os/part4-formula/OpenDocument-v1.3-os-part4-formula.html>.

## Data classification

Synthetic. The language exists only to exercise generation.

## Capabilities demonstrated

- **A grammar as a graph.** Chapters, tokens, productions, operators,
  functions, error kinds, and examples are resources with typed relations:
  a production's `uses` must resolve to tokens or productions, an
  operator's `token` and `production` to their kinds, an example's
  `expected_error` to an error resource.
- **Whole-grammar checks.** Every token is used, every production is
  reachable from the start production, operators on one production share
  a precedence level and productions nest from looser to tighter, every
  error kind and function is exercised by an example, chapter sequences
  are contiguous.
- **Generated code derived from the graph.** `rules/formulo.star` builds
  every table the crate contains: the token enum and the longest-first
  punctuator table, the keyword table, the binary and unary operator
  tables with precedence and associativity, the function registry with
  arities, the error-kind enum, and one test per example. The
  recursive-descent skeleton lives in the templates; the language does not.
  Change `operator-multiply` to precedence 2 and the generated table, the
  check, and the conformance tests all react.
- **Multiple output roots.** Rust sources go to `formulo/src/generated`,
  tests to `formulo/tests`, documents to `generated`. The crate shell
  (`formulo/Cargo.toml`, `formulo/Cargo.lock`, `formulo/src/lib.rs`) is
  hand-written and mounts the generated modules.
- **Provenance in code.** Every generated Rust file starts with the SPDX
  and provenance lines from `bearout.header`; the JSON corpus carries
  provenance only in `bearout-state.toml`.

## Resource model

| Kind | Key fields | Relations |
| --- | --- | --- |
| `chapter` | `title`, `sequence`, `status`, `## Purpose` | none |
| `token` | `kind`, `variant`, `text` or `pattern` | `chapter` |
| `production` | `rule`, `start` | `uses` → token or production, `chapter` |
| `operator` | `name`, `symbol`, `arity`, `precedence`, `associativity` | `token`, `production` |
| `function` | `name`, `min_args`, `max_args` | `chapter` |
| `error` | `variant`, `## Summary` | `chapter` |
| `example` | `source`, `expected_ast` or `expected_error` | `exercises` → production, `functions` → function, `expected_error` → error |

The grammar: a formula is `=` followed by a comparison; comparison, additive,
multiplicative, and power levels nest in that order; unary plus and minus
bind tightest; primaries are numbers, strings, `TRUE`/`FALSE`, cells,
ranges, calls, and parenthesised expressions. Numeric literal spelling is
preserved; nothing is evaluated.

## Generated artifacts

- `formulo/src/generated/ast.rs`, `lexer.rs`, `parser.rs`: the crate.
- `formulo/tests/conformance.rs`: one test per example.
- `generated/syntax-reference.md`: chapters, tokens, productions,
  operators, functions, errors, examples.
- `generated/conformance.json`: the machine-readable corpus.

`mise run check:generated` compiles and tests the crate with the pinned
Rust toolchain and a separate target directory.

## Try breaking it

- Set `precedence = 2` on `operator-multiply`: B015
  `precedence-nesting` on `production-additive`, because it now contains a
  production whose operators bind no tighter.
- Set `precedence = 5` on `operator-add` only: B015 `precedence-level` on
  `operator-add`.
- Remove `"token-caret"` from `production-power`'s `uses`: B015
  `uses-incomplete`.
- Point `example-unknown-function`'s `expected_error` at
  `error-unexpected-token`: the example resource stays valid, but the
  generated test fails, which `mise run check:generated` reports.
- Delete `example-unterminated.md`: B015 `error-coverage` on
  `error-unterminated-string`.
- Rename `token-comma` to `token-sep` without updating `production-call`:
  B009 on the production.

## Sample omissions

No evaluation, coercion, dates, dependency cycles, or Excel compatibility.
No string concatenation operator. No sheet-qualified references.

## Engine gaps

- Bearout compiles the generated crate through `mise run check:generated`,
  not through `bearout generate`. Running a compiler on outputs would be
  process execution, which the kernel does not perform.
- `pattern` fields are documentation; the generated lexer's scanners are
  hand-written in the templates, and nothing proves the two agree.
