# Design

Bearout is a deterministic repository contract engine. A *contract* here is
a machine-checkable agreement about the resources in a repository: their
envelope, their shape, the relations between them, and the artifacts
generated from them. It is not necessarily a legal contract; two of the
samples model commercial and multilateral records, but the engine knows
nothing about law.

## Kernel responsibilities

The Rust kernel owns everything that must be the same for every adopting
repository:

- discovery of resources beneath the declared roots, sorted, without
  following symbolic links;
- parsing of the resource envelope: TOML front matter through `toml_edit`
  over an exact byte range, and the Markdown body through Comrak;
- structural validation against JSON Schema 2020-12 shapes and the
  `x-bearout` vocabulary;
- graph construction: identifier index, typed relations, link and anchor
  resolution;
- the Starlark runtime: contained loading, resource limits, cancellation,
  the ABI, and immutable views;
- diagnostics with stable codes and deterministic ordering;
- generation: plan validation, rendering, provenance, the state manifest,
  and confined, staged delivery through a filesystem capability.

## Repository responsibilities

Everything domain-specific belongs to the repository:

- schema identifiers and their shapes;
- validators, project checks, and generators in Starlark;
- templates;
- naming conventions such as "file stem equals resource id", which the
  samples enforce as policy and the kernel does not.

The kernel never learns a repository's semantics. When two real consumers
need the same mechanism, it can move into the kernel; until then it stays
in policy.

## Phase ordering

Every run proceeds through these phases in order:

1. **bootstrap**: open the project as a capability, parse `bearout.toml`,
   validate the roots;
2. **discovery**: walk the resource roots;
3. **parsing**: envelope, body structure, fragments;
4. **policy load**: the Starlark entry module and everything it loads,
   which registers schemas, checks, and generators;
5. **structural validation**: shape, required sections, fragment shapes;
6. **graph construction**: identifiers from every parsed resource,
   relations and links from structurally valid ones;
7. **repository policy**: validators over structurally valid resources,
   then checks over the whole graph only when no error has been reported;
8. **generation planning**: only when no error has been reported;
9. **rendering**: every artifact into memory, with digests;
10. **delivery**: comparison against the state manifest, then atomic
    per-file replacement.

A resource that fails parsing or structural validation is never handed to a
validator. Its identifiers still resolve, so a reference to it does not
produce a second, cascaded diagnostic. Checks and generators receive only a
structurally valid graph.

## Capability boundary

`bearout.toml` is static and is the security boundary of a project. It
names the entry module and grants four kinds of root: resource roots, the
rules root beneath which `load()` resolves and shapes live, the templates
root, and the output roots. Roots are disjoint, none is the project root,
and the bootstrap itself lies beneath none of them. Repository policy can
register schemas, checks, and generators but cannot widen any grant.

All filesystem access goes through a `cap-std` directory capability opened
on the project root; the kernel holds no ambient path. Output delivery
refuses absolute paths, parent traversal, paths outside the output roots,
symbolic links anywhere in the output path, and files that Bearout does not
own according to the state manifest.

## Schema and resource identity

A schema identifier is `<namespace segments>/<kind>@<major>`, lowercase
kebab-case, at least one namespace segment, positive major. Identifiers
are repository-owned and never centrally registered; the kernel only
requires them to be well formed and registered by the project's policy.

Resource and fragment identifiers share one namespace per project and are
lowercase kebab-case. A fragment's kind is `<schema>#<fragment kind>`, which
is also how a relation names fragment targets.

The envelope keys `schema`, `id`, and `refs` are owned by the kernel and
validated by it. Every other front-matter key is a repository field that a
shape validates. A shape may not declare the envelope keys.

## Starlark ABI

See [starlark-abi.md](starlark-abi.md). The ABI is version 0 and
experimental. Scripts receive frozen dict views and return lists of host
values constructed by `error()`, `warning()`, and `output()`; every field is
checked at construction and again on admission. Nothing is decoded
permissively, and an invalid target is a diagnostic against the script, not
a silently reattributed finding.

## Diagnostic stability

See [diagnostics.md](diagnostics.md). Codes, severities, and report
ordering are the machine-facing surface. Until the first tagged release
they are experimental; after it, a code is never reused for a different
meaning and removals are announced in the changelog.

## Generated-output lifecycle

Generation is staged. Every plan entry is validated, every artifact is
rendered into memory, and digests are computed before anything touches the
tree. `bearout-state.toml` at the project root records, for every output
Bearout owns, the generator, template, content digest, and an input digest
over the Bearout version, the template source, and the context. The
manifest is parsed strictly: it is absent, valid, or invalid, and an invalid
manifest stops the run before any file is touched. A manifest with no
`outputs` entries omits the key, which is the serializer's own form for an
empty manifest and is accepted; every other omission is an error.

Ownership is proven only by the manifest. An existing file at a planned
path that the manifest does not own is never overwritten, even when its
bytes already equal the rendered bytes; generation must begin with the
path absent for ownership to be established.

Delivery is one journaled transaction: changed outputs are written, owned
and unmodified orphans are removed, then the manifest is written. Each file
replacement is atomic: an exclusively created, uniquely named temporary
file in the same directory is written through its open handle, its data is
synced, and it is renamed into place, so a symbolic link is never followed
or installed and a reader sees either the old or the new file. That is the
whole guarantee. The multi-file sequence is not atomic: if a step fails,
every completed step is undone from the journal where possible, both the
failure and any restoration failure are reported, and the manifest is never
written, so the manifest never claims a delivery that did not complete.
This is in-process rollback, not crash consistency. Bearout does not sync
directories, so a power failure or kill between two renames can leave some
outputs new and some old with the previous manifest still in place; the
next `bearout generate --check` reports exactly that as stale outputs, and
a normal run repairs it because every such file is still owned.

Reads of rules and shapes refuse paths that pass through a symbolic link.
Templates are read through the templates root capability and may be
symbolic links inside the project; the capability confines where they can
point. Links whose target carries a URL scheme, including single-letter
schemes such as `c:`, are not resolved against the tree.

`bearout generate --check` reports missing, stale, orphaned, and re-owned
outputs and a stale state manifest. A normal run removes an orphan only when
the state manifest proves Bearout produced that exact path and the file
still carries the recorded digest. Untracked or modified files are never
deleted or overwritten.

Outputs in comment-capable formats carry an SPDX line when the bootstrap
declares a license and always carry a provenance line; the kernel rejects
an output that lacks them. Formats without comments carry provenance only
in the state manifest.

## Why Starlark and JSON Schema

See [technology-evaluation.md](technology-evaluation.md) for the
comparison with Nickel, CUE, and the previous Rhai runtime. In short:
Starlark is deterministic, embeddable in Rust with a contained loader,
linting, static typechecking, cancellation, and execution, heap, and
call-stack limits; JSON Schema 2020-12 is the widely understood declarative
shape layer that editors already support. Nickel could replace both the
bootstrap and the shapes but would overlap with Starlark as a second rule
language; CUE is strong prior art whose reference implementation is Go and
does not fit a single Rust binary.

## Rendering limits

Starlark evaluation is bounded by ticks, heap, and call-stack depth.
Rendering is bounded too: every output renders under `MiniJinja` fuel
(`limits.template_fuel`, measured from the samples with headroom) and into a
writer that stops at `limits.output_bytes` (a conservative bound, not
measured) before an unbounded buffer can be allocated. Exceeding either is
B019 and touches nothing in the tree.

## Trust limitations

Bearout is a capability-confined host with resource limits. It is not a
sandbox for hostile repositories: the limits bound runaway policy code and
the capability confines writes, but checking a repository from an untrusted
author is not a supported security boundary. See `SECURITY.md`.
