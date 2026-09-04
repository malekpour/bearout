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

## Git access: the `git` subprocess

The index and revision sources read Git through the `git` executable,
invoked with an argument vector: `rev-parse` to locate the repository and
resolve names, `ls-files --stage` and `ls-tree -r -t -l` to capture
entries, `diff-index --cached` with and without `--ita-invisible-in-index`
to identify intent-to-add entries, and a long-lived `cat-file --batch` to
load blobs by identity, all against one private copy of the index.
Nothing is interpolated into a shell, the environment cannot redirect the
repository, its objects, or its configuration, replacement objects and
lazy fetching are disabled, listings and blobs are bounded, and Git's
error output is reduced to one sanitized line.

Alternatives considered:

- **gitoxide** (`gix`) would remove the runtime dependency on `git`, but at
  the cost of a large dependency tree, a still-moving API, and the burden
  of tracking index extensions (split and sparse indexes, untracked cache,
  file-system monitor data), linked worktrees, object-format transitions,
  and revision syntax ourselves. Every one of those is exactly what the
  installed Git already handles.
- **libgit2** (`git2`) adds a C build dependency, has no SHA-256
  repository support, and lags Git in index and worktree features.

A repository that is checked through Git already has Git; requiring it for
the Git-backed sources costs nothing there, keeps the binary small, and
makes Git itself the authority on what the index and a revision contain.
The Git-backed sources are documented as experimental so the choice can be
revisited when real integrations produce evidence.

The comparison baseline reuses the revision source unchanged; only its
capture tolerates a revision that predates the project directory. Change
facts are computed by Bearout from the bytes it parsed, not by `git diff`:
Git's rename and copy detection is a similarity heuristic whose result
depends on thresholds and on the rest of the tree, while contract policy
needs a fact that two runs, two machines, and two candidate sources agree
on. Resources keep their identity through their ids, so policy can pair
a moved record without the kernel guessing.

## Text hygiene: `ec4rs` for EditorConfig

Repositories already declare their common text policy in
`.editorconfig`, so Bearout reads that rather than inventing a vocabulary.
Three Rust options were considered: `ec4rs` (a from-scratch
implementation of the specification, Apache-2.0, no C dependency, small
dependency tree, able to parse from any reader), `editorconfig-core`
(FFI bindings to the C library, adding a native build dependency), and
`editorconfig` (an older crate with less complete matching). `ec4rs` was
chosen because its parser accepts bytes from a `ReadTree` and its section
matching applies to a path made relative to the configuration file's
directory, which is exactly what a source-exact resolver needs: Bearout
never lets it open files from the live filesystem. Bearout enforces only
`charset`, `end_of_line`, `insert_final_newline`, and
`trim_trailing_whitespace`, and says so; it does not claim complete
EditorConfig compatibility, and a supported property with a value it
cannot enforce is a diagnostic rather than a guess.

## External formatters: a byte-transform protocol

Syntax-aware formatting is delegated to repository-pinned programs
through the narrowest protocol formatters commonly support: exact bytes on
standard input, canonical bytes on standard output, a filename hint as an
argument. Bearout does not embed any language's formatter, does not parse
tool-specific diagnostics, and does not run linters; the first would tie
the kernel to a language, the second and third need a design of their
own. Programs run from an argument vector in a private working directory
seeded from the selected tree, with bounded streams and a timeout, and
only under explicit host authorization, because they are trusted host
processes rather than confined scripts. Bearout does not read `mise.toml`
or detect tool versions: the calling repository runs Bearout inside the
environment that pins its tools.

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
