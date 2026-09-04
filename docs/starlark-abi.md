# Starlark ABI

**ABI version 0. Experimental.** Nothing here is a compatibility promise
until the first external integrations have exercised it. Breaking changes
bump the version and are recorded in the changelog.

Bearout evaluates repository policy in Starlark using starlark-rust's
extended dialect (type annotations and f-strings on top of the Starlark
specification). Scripts receive immutable views and return host values.
They have no filesystem, environment, network, clock, or random access,
and they do not learn which source (working directory, Git index, or
revision) a run reads: the views are the same for every source. Nor do
they have process access: the external formatters a repository declares
run only under host authorization, are never callable from a script, and
leave no trace in any view.

## Entry module

`bearout.toml` names one entry module, `entry = "bearout.star"`. It runs
once per Bearout run with these registration functions in scope:

| Function | Meaning |
| --- | --- |
| `schema(id, shape=None, validate=None)` | Register a schema identifier. `shape` is a `.schema.toml` path relative to the rules root. `validate` is a function of one resource view. |
| `check(name, function)` | Register a project-level check: a function of one project view. |
| `generator(name, function)` | Register a generator: a function of one project view that returns outputs. |

Registration functions exist only in the entry module. Loaded modules that
call them fail at load time. Registering the same id or name twice fails.
Schema identifiers must match `<namespace>/<segments>/<kind>@<major>` in
lowercase kebab-case; names and codes must be lowercase kebab-case.

## Loading modules

`load("path/to/module.star", "symbol")` resolves beneath the rules root
declared by `[rules] root`. A load path must be relative, normalized (no
`.`, `..`, empty segments, or backslashes), and end in `.star`. Absolute
paths, escapes, paths through symbolic links, missing modules, and cycles
are B012 errors that name the import chain. Each module is parsed, linted
(B018 warnings), statically typechecked (B012 errors), evaluated once under
the resource limits, and frozen.

## Host constructors

Available in every module:

| Constructor | Result |
| --- | --- |
| `error(message, resource=None, path=None, side="candidate", line=None, code=None)` | A finding that fails the run (B015). |
| `warning(message, resource=None, path=None, side="candidate", line=None, code=None)` | A finding that does not fail the run (B016). |
| `output(template, path, context=None)` | One planned file: `template` relative to the templates root, `path` relative to the project root, `context` a dict. |

Rules enforced at construction, inside the script, so the error names the
call site:

- only the fields above; a misspelled keyword is a type error;
- `message` is a non-empty string; `line` is a positive integer; `code` and
  `resource` are lowercase kebab-case identifiers;
- a finding's `path` is a normalized project-relative path of a schema-less
  document; `resource` and `path` are mutually exclusive;
- a finding's `side` is exactly `"candidate"` (the default) or
  `"baseline"`;
- an output's `path` and `template` are normalized relative paths;
- `context` converts to JSON; anything else is an error.

Rules enforced by the kernel when the value is admitted (B014 if violated):

- a validator may omit `resource` or name its own resource only; it may
  never name a `path` or the baseline side;
- a check must name a `resource` that exists in the valid graph of the
  selected side or a `path` that exactly matches a parsed document of that
  side; `side="baseline"` is valid only when the run was given a
  comparison baseline;
- `line` is at most the line count of the named resource or document on
  the selected side;
- a resource present on both sides is never targeted ambiguously: the
  side names which tree, and therefore which path, the finding is about;
- a finding on the baseline side is reported with the structured
  `baseline` side (a `baseline:` prefix in text output).
- an output path lies beneath a declared output root and does not collide
  with another output after normalization and case folding.

Every callback returns a list. A validator or check returns a list of
findings; a generator returns a list of outputs. Any other value, or any
other item type, is B014. `print()` output is captured as a B017 warning.

## Resource view

A dict with these keys, all frozen:

| Key | Value |
| --- | --- |
| `id`, `schema`, `path` | Strings. `path` is project-relative with forward slashes. |
| `refs` | List of untyped reference ids. |
| `fields` | Dict of repository-owned front-matter fields. TOML dates are strings in TOML text form. |
| `body` | The Markdown body byte-for-byte. |
| `sections` | List of `{level, title, anchor, line, text}` in document order. `anchor` is the GFM heading anchor. |
| `anchors` | List of `{id, line}` for explicit `<a id>` and `<a name>` anchors in raw HTML. |
| `blocks` | List of fenced blocks `{lang, attrs, content, line, section}`. |
| `fragments` | List of `{kind, id, fields, line, section}` for blocks tagged `bearout=<kind>`. |
| `links` | List of `{target, text, line}`; `text` is the visible link text flattened to plain text. |
| `images` | List of `{target, alt, line}`; `alt` is the alt text flattened to plain text. |
| `relations` | Dict from relation field name (including `refs`) to the list of ids it names, as written. |
| `referenced_by` | List of `{from, field}` for every relation that names this resource or one of its fragments. |

Line numbers are one-based lines of the resource file. `section` is an
index into `sections` or `None`.

## Project view

| Key | Value |
| --- | --- |
| `resources` | List of resource views in path order. |
| `by_id` | Dict from resource id to view. |
| `by_schema` | Dict from schema id to the list of resource ids in path order. |
| `ids` | Dict from every id, resource or fragment, to its kind: the schema id, or `schema#kind` for a fragment. |
| `documents` | List of schema-less document views in path order; empty when the bootstrap selects no documents. |
| `comparison` | `None` unless the run was given a comparison baseline; otherwise the comparison view below. |

## Comparison view

**Experimental**, like the Git-backed sources it depends on. When a run
names a baseline revision, `project["comparison"]` is a dict:

| Key | Value |
| --- | --- |
| `baseline` | The historical project: `revision` as supplied, `tree` (the resolved tree identity), `digest` (the deterministic digest of the captured tree), and `resources`, `by_id`, `by_schema`, `ids`, and `documents` with exactly the shapes of the candidate project view. |
| `changes` | List of `{path, change, before, after}` in path order over the contract surface of both sides: the bootstrap, the discovered resources, and the discovered schema-less documents. `change` is `added`, `removed`, or `modified`; `before` and `after` are `None` or `{classification, digest, bytes}` with `classification` one of `resource`, `document`, `manifest`. |

The candidate is the top-level project view; the comparison never replaces
it. The baseline is projected through the candidate's policy: its own
`bearout.toml` decided which paths it classified as resources and
documents, the candidate's limits bound it, and the candidate's registered
schemas and shapes validated it. Only structurally valid baseline
resources and parsed baseline documents appear. No baseline Starlark,
generator, or template ever runs, and nothing gives a script access to
arbitrary historical blobs, the working filesystem, Git, or the source a
run reads. Change facts compare exact bytes by path: a rename is a removal
plus an addition, a reclassified path is a modification, and equal content
yields no facts. What is protected, immutable, or permitted to change is
entirely the repository policy's decision; the kernel enforces nothing.

## Document view

A schema-less document selected by `[documents]` has Markdown structure
but no envelope, schema, identifier, shape, or relations, and none is
synthesized. Only documents that were read and parsed appear; a document
reported as B022 does not.

| Key | Value |
| --- | --- |
| `path` | Project-relative path with forward slashes. |
| `text` | The whole document, with a leading byte-order mark removed. |
| `line_count` | Number of lines. |
| `sections` | As in the resource view. |
| `anchors` | As in the resource view. |
| `links` | As in the resource view. |
| `images` | As in the resource view. |

Views are the same whichever source the run reads; nothing in them names
the working directory, the index, or a revision.

Only structurally valid resources appear. A resource that failed envelope
parsing, shape validation, required sections, or fragment validation is
never passed to a validator, and checks and generators run only when the
whole run is free of errors so far.

## Template context

The kernel adds a `bearout` dict to every output context:

| Key | Value |
| --- | --- |
| `version` | Bearout version. |
| `generator`, `template`, `output` | The plan entry. |
| `license` | The `[outputs] license` value or `None`. |
| `header` | Lines to emit as a comment header: the SPDX line when a license is configured, then the provenance line. |

Outputs whose extension supports comments must contain the provenance
header within their first five lines; the kernel rejects them otherwise
(B019). Formats without comments, such as JSON and CSV, carry provenance
only in `bearout-state.toml`.

Templates are MiniJinja with strict undefined handling: an undefined
variable is an error, not empty text. MiniJinja's default auto-escaping
applies by template name after the `.j2` suffix is removed, so `{{ x }}` in
a `*.json.j2` template renders a JSON literal; the samples use the `tojson`
filter explicitly for whole documents. Filters `snake_case`, `kebab_case`,
`screaming_case`, `pascal_case`, `camel_case`, and `quote` (a JSON string
literal) are provided.

## Limits

Every evaluation runs under the bootstrap `[limits]`: `ticks`,
`heap_bytes`, and `call_stack`, plus cancellation from the host. Exceeding
a limit is B013. Rendering is bounded separately by `template_fuel` and
`output_bytes` (B019). The defaults are operational bounds derived from the
samples in this repository, not a proof that a hostile repository is
contained.
