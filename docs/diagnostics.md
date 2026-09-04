# Diagnostic codes

Every finding carries a stable code, a severity, a project-relative path
with forward slashes, an optional one-based line, an optional
repository-owned rule identifier, and a message. Reports are sorted by
path, code, line, rule, and message, then deduplicated.

## Stability policy

Codes are experimental until the first tagged release. After that:

- a code is never reused for a different meaning;
- new codes are appended;
- a retired code is announced in the changelog and never reassigned;
- messages are not part of the stable surface; match on codes.

## Exit codes

| Exit | Meaning |
| --- | --- |
| 0 | The run completed and produced no error-severity finding. |
| 1 | The run completed and produced at least one error-severity finding. |
| 2 | Invocation, configuration, source, or engine failure: the report's `fatal` field explains. |

Source failures are fatal: a Git-backed source that cannot be opened (no
repository, no `git` executable, an unmerged index), a revision that does
not resolve, writing generation requested against the index or a
revision, and a comparison baseline that cannot be opened, does not
resolve, has an unusable historical `bearout.toml`, or names roots and
files its own tree lacks. Conflicting source flags are an invocation
error. A Git-backed run reports its diagnostics with the same codes,
paths, and ordering as a working-directory run; only the tree the paths
refer to differs.

With a comparison baseline, diagnostics about the historical tree keep
their codes (B001, B002, B003, B005, B008, B022, and so on, plus B015 and
B016 for policy findings targeted at the baseline) and carry a
`side` of `baseline`: in JSON as `"side": "baseline"` (absent for the
candidate), in text as a `baseline:` prefix before the path. Every
candidate diagnostic sorts before every baseline diagnostic. A baseline
diagnostic of error severity fails the run like a candidate one, because
history the current policy cannot interpret is a comparison the policy
cannot make.

`--format json` prints one JSON report for every outcome, including fatal
ones. With a comparison, the report also carries `baseline`, the resolved
baseline identity in the same shape as `source` (`kind`, `revision` as
supplied, `tree`, `digest`); it is absent when no baseline was requested.
 Its `outputs` list is non-empty only when generation succeeded: in
write mode the outputs delivered or already current, in check mode the
outputs verified as current. A failed rendering, state validation, check,
or delivery leaves it empty, and `check` runs never fill it. For the Git
sources, a completed run also carries an experimental `source` object with
`kind` (`index` or `revision`), for a revision the `revision` name as
given and the resolved `tree` identity, and for both a deterministic
`digest` of the captured entries beneath the project (`blake3:` followed
by 64 hexadecimal characters; equal for identical content from either
source; not a Git object identity). The field is absent for the working
directory and for fatal outcomes.

## Catalog

| Code | Severity | Meaning |
| --- | --- | --- |
| B001 | error | A resource or shape file could not be read, or a resource exceeds `limits.resource_bytes`. Template failures are B019 and Starlark loading failures are B012. |
| B002 | error | The resource envelope is malformed: front matter, TOML, or a reserved key. |
| B003 | error | A schema identifier is malformed or names a schema the policy did not register. |
| B004 | error | A shape file is not a usable JSON Schema 2020-12 document or its `x-bearout` vocabulary is invalid. |
| B005 | error | Front matter or a fragment violates its declared shape. |
| B006 | error | A section the shape requires is missing from the body. |
| B007 | error | A fenced fragment is malformed or of an undeclared kind. |
| B008 | error | The same identifier is defined more than once. |
| B009 | error | A reference names an identifier that nothing defines. |
| B010 | error | A typed relation resolves to a node of the wrong kind. |
| B011 | error | A Markdown link or image, in a resource or a schema-less document, points at a missing file, names an anchor its target does not define, names an anchor in a Markdown file that is neither a resource nor a selected document, or escapes the project; an image may not name a directory. |
| B012 | error | A Starlark module could not be loaded, parsed, resolved, or typechecked. The message names the import chain. |
| B013 | error | A Starlark call failed, was cancelled, or exceeded a resource limit. |
| B014 | error | A Starlark call returned a value the ABI does not accept, or a finding with an invalid target: an unknown resource or document, a validator naming another resource or a document, or a line past the end. |
| B015 | error | An error reported by repository policy through `error()`. |
| B016 | warning | A warning reported by repository policy through `warning()`. |
| B017 | warning | A script printed text. |
| B018 | warning | A Starlark lint finding. |
| B019 | error | A generation plan entry is invalid, its template is missing or unreadable, its context holds a number no template value can represent, rendering failed or exceeded `limits.template_fuel` or `limits.output_bytes`, or the provenance header is absent. |
| B020 | error | A generated output is missing, stale, unowned, orphaned, or changed ownership, or the state manifest is out of date or invalid. |
| B021 | error | Delivering a generated output failed (with restoration attempted and reported), or delivery was refused to protect a file Bearout does not own. |
| B022 | error | A schema-less document selected by `[documents]` could not be read, is not valid UTF-8, or exceeds `limits.document_bytes`. |
| B023 | error | Hygiene configuration: an `.editorconfig` of the selected tree cannot be read or parsed (reported once, on that file), or a property it sets for a selected file has a value Bearout cannot enforce (reported on the selected file). Files governed by an unusable `.editorconfig` are not checked. |
| B024 | error | A file selected by `[hygiene]` could not be read or exceeds `limits.file_bytes`. |
| B025 | error | A selected text file is not valid UTF-8, begins with a byte-order mark that `charset = utf-8` forbids, or lacks the mark that `charset = utf-8-bom` requires. Nothing else is checked in that file. |
| B026 | error | A line terminator contradicts `end_of_line`. One per file, naming the first line. |
| B027 | error | A non-empty file does not end with exactly one newline under `insert_final_newline = true`, or ends with one under `false`. |
| B028 | error | A line ends with spaces or tabs under `trim_trailing_whitespace = true`. One per file, naming the first line. |

## Repository rule identifiers

Policy may attach a `code` to `error()` and `warning()`. It is recorded as
the `rule` field and rendered in brackets after the code, as in
`B015[ruling-sequence]`. Rule identifiers are lowercase kebab-case and are
owned by the repository; Bearout assigns no meaning to them.
