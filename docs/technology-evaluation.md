# Technology evaluation

This note records why the current stack was selected and which
alternatives were considered. It is deliberately short; the decisions can be
revisited when the first external integrations produce evidence.

## Procedural policy: Starlark

Starlark, through `starlark-rust`, is the selected language for validators,
project checks, and generation planning because it is:

- **deterministic**: no ambient filesystem, environment, network, clock,
  or randomness; the host decides what a script can reach;
- **embeddable in Rust** as a library with no external runtime;
- **containable**: `load()` goes through a host-provided loader, so imports
  resolve only beneath the rules root and cycles and escapes are rejected;
- **analyzable**: the dialect carries a linter and a static typechecker,
  and this host runs both on every module;
- **bounded**: the evaluator enforces execution ticks, heap size, and
  call-stack depth, and checks a cancellation flag.

The previous runtime was Rhai. It worked, but it offered no static analysis,
no module containment beyond disabling `import`, and a permissive value
model in which every result was a dynamically typed map decoded by
convention. The migration to Starlark removed Rhai entirely; the repository
has one policy runtime.

Not adopted: `starlark_lsp`. It is a possible editor component, not a
kernel dependency.

## Declarative shapes: JSON Schema 2020-12

JSON Schema remains the declarative layer for the structure of front matter
and fragments. It is widely understood, editors and TOML tooling already
consume it, and the `jsonschema` crate implements 2020-12 with format
validation. Bearout adds a small `x-bearout` vocabulary for what JSON Schema
cannot express: typed relations, required headings, and fragment kinds. The
vocabulary is validated against a kernel meta-schema so that a misspelled
extension is an error.

## Nickel

Nickel is a credible alternative. Its contracts could replace both the
bootstrap configuration and JSON Schema, and its evaluation model is well
suited to validated configuration. Combining it with Starlark, however,
would give repositories two overlapping rule languages with different
semantics for the same kinds of constraint. It was therefore not added in
this pass. It should inform how the shape vocabulary evolves, in
particular the idea of contracts attached to fields rather than to whole
documents.

## CUE

CUE is strong prior art for constraints and configuration unification. Its
official implementation is Go, and embedding it would conflict with the
single-Rust-binary direction: it would mean either a second process or a
foreign-function boundary. It was not added. Its ideas about lattice-based
unification and definitions are worth borrowing in the internal model.

## Scope of these alternatives

Nickel and CUE inform the internal model. They are not to become a
speculative public backend or plugin framework; Bearout does not need a
pluggable rule engine until a real consumer demonstrates the need.

## Other components

- `toml_edit` parses the bootstrap, front matter, header-only resources,
  and shapes, preserving spans for diagnostics and formatting for future
  editing operations.
- Comrak provides a CommonMark and GFM abstract syntax tree with source
  positions and the GitHub anchor algorithm.
- MiniJinja renders templates in strict mode with a loader confined to the
  templates root.
- `cap-std` is the filesystem capability; the kernel opens the project
  root once and never uses an ambient path afterwards.
- `cap-tempfile`, from the same project, provides the per-file delivery
  primitive: a uniquely named temporary file created exclusively in the
  output's own directory, written through its handle, and renamed into
  place, with platform-specific replacement on Windows and cleanup on drop.
  It replaced a hand-written predictable-name temporary that a pre-existing
  symbolic link could redirect. Its randomness never reaches Starlark.
- `spdx` validates `[outputs] license` as an SPDX expression so that the
  header stamped into generated files is well formed; inventing a partial
  SPDX grammar would have been worse than a small, established dependency.
- BLAKE3 provides content and input digests for generated-output
  provenance.
