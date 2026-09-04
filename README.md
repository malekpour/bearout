# Bearout

Bearout is a deterministic repository contract engine. A repository keeps
prose and structured metadata together as resources; Bearout discovers
them, validates their shape, resolves their relationships, applies the
repository's own rules, and generates artifacts from the verified graph,
with provenance.

A *contract* here is a machine-checkable agreement about the resources in a
repository. It is not necessarily a legal contract.

> [!WARNING]
> Bearout is an experiment. Its bootstrap, resource envelope, shape
> vocabulary, Starlark ABI (version 0), diagnostic codes, and generated
> outputs are not stable yet. Do not build a compatibility promise on the
> current syntax.

## Two layers

- The **Rust kernel** owns discovery, parsing, graph construction,
  diagnostics, resource limits, path confinement, and filesystem writes.
- The **repository** owns every domain rule: schema identifiers and their
  JSON Schema shapes, validators, project checks, and generation plans in
  Starlark, and templates.

A schema identifier such as `example/decision-records/decision@1` belongs to
the repository that defines it. Nothing is registered with Bearout or
compiled into the binary. See [`docs/design.md`](docs/design.md) for the
responsibilities, phases, and boundaries.

## The bootstrap

`bearout.toml` is static and is the capability boundary of a project:

```toml
version = 1
entry = "bearout.star"

[resources]
roots = ["records"]

[rules]
root = "rules"          # `load()` resolves here; shapes live here

[templates]
root = "templates"

[outputs]
roots = ["generated"]   # the only places generation may write
license = "Apache-2.0"  # stamped into generated headers

[limits]                # optional; see docs/design.md for which defaults are measured
ticks = 1000000
template_fuel = 2000000
```

Repository policy can register schemas, checks, and generators. It cannot
widen the roots the bootstrap grants. Roots are disjoint, none is the
project root, and all filesystem access goes through a capability opened on
the project root.

## Repository policy

The entry module registers what the project uses:

```python
load("decision.star", "validate_decision")
load("log.star", "check_supersession_is_reciprocal")
load("decision-index.star", "plan_decision_index")

schema("example/decision-records/decision@1",
       shape = "decision.schema.toml", validate = validate_decision)
check("supersession-is-reciprocal", check_supersession_is_reciprocal)
generator("decision-index", plan_decision_index)
```

A validator receives one frozen resource view and returns findings; a check
receives the project view; a generator returns outputs:

```python
def validate_decision(resource):
    if resource["fields"]["status"] == "accepted" and not rulings(resource):
        return [error("an accepted record must carry a ruling", code = "rulings-required")]
    return []
```

`load()` resolves only beneath the rules root; escapes, symbolic links,
and cycles are rejected with the import chain. Every module is linted and
statically typechecked, and every evaluation runs under tick, heap, and
call-stack limits with cancellation. Scripts have no filesystem,
environment, network, clock, or random access. The full ABI is in
[`docs/starlark-abi.md`](docs/starlark-abi.md).

## Resources and shapes

A resource is Markdown with TOML front matter, or a header-only TOML file:

````markdown
+++
schema = "example/decision-records/decision@1"
id = "decision-0004"
title = "Records are numbered when merged"
status = "accepted"
date = "2026-09-02"
supersedes = ["decision-0002"]
+++

# Records are numbered when merged

## Context

### decision-0004-ruling-01

```toml bearout=ruling
id = "decision-0004-ruling-01"
text = "A record receives its sequence number when it is merged."
```
````

`schema`, `id`, and `refs` are the envelope keys the kernel owns. Every
other key is validated by the schema's shape, a JSON Schema 2020-12
document authored in TOML with a small `x-bearout` vocabulary:

```toml
"$schema" = "https://json-schema.org/draft/2020-12/schema"
type = "object"
additionalProperties = false
required = ["title", "status", "date"]

[properties.supersedes]
type = "array"
items = { type = "string" }
"x-bearout" = { ref = "example/decision-records/decision@1" }  # typed relation

["x-bearout"]
sections = ["Context"]                                          # required heading

["x-bearout".fragments.ruling]                                  # a fragment kind
type = "object"
required = ["id", "text"]
```

The vocabulary itself is validated: an unknown `x-bearout` key, a relation
on a non-string property, or a shape that declares an envelope key is an
error. Markdown bodies are parsed with Comrak; headings get GFM anchors,
fenced blocks tagged `bearout=<kind>` become typed fragments with
project-wide identifiers, and every relative link and `#anchor` is resolved.

## Phases

bootstrap, discovery, parsing, structural validation, graph construction,
repository policy, generation planning, rendering, delivery. A resource that
fails parsing or structural validation is never passed to a validator, and
its identifiers still resolve so nothing cascades. Checks run only on an
error-free graph; generation runs only on an error-free project.

## Generation

Scripts never write files. A generator returns `output(template, path,
context)` entries; the kernel validates every path against the output
roots, renders every artifact into memory with MiniJinja in strict mode,
computes BLAKE3 digests, and only then delivers each file through an atomic
rename. `bearout-state.toml` records which outputs Bearout owns and their
provenance; `bearout generate --check` reports missing, stale, unowned,
orphaned, and re-owned outputs. A file the manifest does not own is never
overwritten, even when its bytes already match; orphans are removed only
when the manifest proves Bearout wrote them and they are unmodified. The
report's `outputs` list names delivered or verified files only when
generation succeeded. Third-party content is recorded in
[`NOTICE.md`](NOTICE.md).

## Commands

```sh
bearout check [path]                    # exit 0 clean, 1 findings, 2 fatal
bearout generate [path]                 # check, then deliver outputs
bearout generate --check [path]         # check, then verify committed outputs
bearout --format json check [path]      # one JSON report for every outcome
bearout check --index [path]            # check what a commit would record
bearout check --revision v1.2 [path]    # check one commit, tag, branch, or tree
bearout generate --check --index [path] # verify outputs as staged
```

Diagnostics use stable codes and forward-slash project-relative paths on
every platform; the catalog and its stability policy are in
[`docs/diagnostics.md`](docs/diagnostics.md).

## Sources

> [!WARNING]
> The Git-backed sources are experimental and require the `git`
> executable on `PATH`. Their flags, semantics, and report fields may
> change.

Without a selection, Bearout reads the live working directory through its
filesystem capability, as before; it makes no snapshot, so concurrent edits
are visible to a run. Two read-only sources read a Git tree instead, and
every input of the run comes from that tree and nothing else: the
bootstrap, the entry module and everything it loads, shapes, resources,
templates, `bearout-state.toml`, the generated outputs that
`generate --check` verifies, and the files that links resolve against. A
file that exists only in the working directory never satisfies a lookup in
a Git-backed run.

- `--index` reads the Git index of the repository that owns the project,
  from one private copy taken when the run starts: the tree a commit would
  record. Staged additions and modifications are present; unstaged
  modifications, untracked files, staged deletions, and intent-to-add
  entries are absent; a staged rename appears only at its destination. An
  unmerged index is a fatal outcome. In a partial-commit hook, Git's
  `GIT_INDEX_FILE` is honoured when it is a regular file directly inside
  the repository's own Git directory.
- `--revision <rev>` reads one commit, tag, branch, or tree object. The
  name is resolved exactly once, at the start; the resolved tree identity
  is recorded in the JSON report's `source` field and used for the whole
  run even if the branch moves meanwhile. An unknown name is a fatal
  outcome.

Either way the JSON report's `source` carries a deterministic `digest` of
the captured entries, equal for identical content from either source, so
a report can be tied to exactly what it examined. Git runs with a fixed
environment: variables that redirect the repository, its objects, or its
configuration are dropped, replacement objects and lazy fetching are
disabled, and nothing is fetched or written.

The project may sit below the repository root, including in a linked
worktree; only paths beneath the project are exposed, and a symbolic link
in a Git tree resolves only inside the tree it is read from. Submodules are
never entered. Blobs are read exactly as Git stores them, without
line-ending conversion or filters, and nothing is checked out.

Both sources are read-only: `check` and `generate --check` accept them,
`generate` without `--check` refuses them with exit code 2. Git support does
not make Bearout a security sandbox, and this phase exposes no history,
diffs, or immutability rules to repository policy; scripts do not learn
which source a run reads.

## Samples

The repository's [`samples/`](https://github.com/malekpour/bearout/tree/main/samples)
directory holds eight complete projects, from three linked notes to a
spreadsheet-expression language whose Rust lexer, parser, and conformance
tests are generated from the resource graph and compiled in CI. The
[samples index](https://github.com/malekpour/bearout/blob/main/samples/README.md)
is the capability matrix. Every sample is checked, and its outputs
verified, by the test suite. Samples are not part of the crates.io package.

## Development

All required tools and versions are pinned in
[`mise.toml`](https://github.com/malekpour/bearout/blob/main/mise.toml).

```sh
mise run setup
mise run fmt
mise run check
```

See [`CONTRIBUTING.md`](CONTRIBUTING.md), [`docs/design.md`](docs/design.md),
and [`docs/technology-evaluation.md`](docs/technology-evaluation.md).

## Trust

Bearout is a capability-confined host with resource limits. It is not a
sandbox for hostile repositories; see [`SECURITY.md`](SECURITY.md).

## Licence

Licensed under the [Apache License 2.0](LICENSE).

Copyright 2026 Ali Malekpour.
