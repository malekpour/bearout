# Starlark ABI

**ABI version 0. Experimental.** Nothing here is a compatibility promise
until the first external integrations have exercised it. Breaking changes
bump the version and are recorded in the changelog.

Bearout evaluates repository policy in Starlark using starlark-rust's
extended dialect (type annotations and f-strings on top of the Starlark
specification). Scripts receive immutable views and return host values.
They have no filesystem, environment, network, clock, or random access.

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
| `error(message, resource=None, line=None, code=None)` | A finding that fails the run (B015). |
| `warning(message, resource=None, line=None, code=None)` | A finding that does not fail the run (B016). |
| `output(template, path, context=None)` | One planned file: `template` relative to the templates root, `path` relative to the project root, `context` a dict. |

Rules enforced at construction, inside the script, so the error names the
call site:

- only the fields above; a misspelled keyword is a type error;
- `message` is a non-empty string; `line` is a positive integer; `code` and
  `resource` are lowercase kebab-case identifiers;
- `path` and `template` are normalized relative paths;
- `context` converts to JSON; anything else is an error.

Rules enforced by the kernel when the value is admitted (B014 if violated):

- a validator may omit `resource` or name its own resource only;
- a check must name a `resource` that exists in the valid graph;
- `line` is at most the line count of the named resource;
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
| `blocks` | List of fenced blocks `{lang, attrs, content, line, section}`. |
| `fragments` | List of `{kind, id, fields, line, section}` for blocks tagged `bearout=<kind>`. |
| `links` | List of `{target, line}`. |
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
