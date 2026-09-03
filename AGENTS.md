# AGENTS.md — working rules for Bearout

Bearout turns repository contracts into diagnostics and generated
artifacts. A plausible invention in the engine can therefore spread silently
into every adopting repository. Keep that leverage visible.

## Core rules

1. Do not invent a repository's semantics in the Bearout kernel. Schema
   names, required fields, document classes, naming conventions, and
   generators belong to repository policy unless more than one real
   consumer demonstrates a stable common requirement.
2. Do not stabilize the experimental bootstrap, resource envelope,
   `x-bearout` vocabulary, or Starlark ABI accidentally. A test demonstrates
   current behaviour; it is not a stability promise. User-facing documents
   must describe the current compatibility status honestly.
3. Keep discovery, diagnostics, and generated output deterministic. Sort
   filesystem input and map traversal explicitly. A clean checkout with the
   same inputs and Bearout version must produce byte-identical artifacts.
4. Treat repository policy as powerful input, not as harmless
   configuration. Preserve the resource limits, the contained loader, and
   the filesystem capability. Do not expose filesystem, process,
   environment, network, clock, or random access to Starlark without a
   documented threat model and explicit approval.
5. Do not describe Bearout as a security sandbox. It is a
   capability-confined host with resource limits; checking repositories
   from untrusted authors is not a supported security boundary.
6. Fail closed on malformed configuration and unregistered schemas. Report
   independent findings together, but never pass a resource that failed an
   earlier phase to a later one, and never restate a consequence of an
   earlier failure as a new diagnostic.
7. Diagnostic codes, severities, and ordering are the machine-facing
   surface. Change them deliberately, document them in
   `docs/diagnostics.md`, and cover the change with tests.
8. Generated artifacts are committed only where a repository treats them as
   reviewed deliverables. Generators must identify their origin, record
   provenance in the state manifest, and be reproducible. Never delete or
   overwrite a file the state manifest does not prove Bearout owns.
9. Prefer a small end-to-end slice driven by a sample over a speculative
   framework. Native plugins, remote schema registries, a universal schema
   language, and a second policy language are out of scope until evidence
   requires them.
10. Preserve SPDX headers and third-party notices.

## Repository practice

- Use the exact tools pinned in `mise.toml`; do not substitute system tools
  or install tools globally.
- Run `mise run fmt` for mechanical formatting and `mise run check` before
  proposing a change.
- Commit `Cargo.lock`; Bearout is an application as well as a library. Use
  `--locked` for Cargo in CI.
- Add a focused test under `tests/` for every bug fix and every change to
  parsing, graph resolution, script limits, diagnostics, or generation. Use
  `tests/fixtures/` or a test-built minimal project, not a copy of a sample.
- Avoid `unsafe`. The lint denies it; the only admitted unsafe code is the
  `ProvidesStaticType` derive that starlark-rust requires for host values.
- Keep commits small, use Conventional Commit headers, and sign off with
  `git commit -s`; the `commit-msg` hook enforces both.
- Record substantial AI assistance in the pull-request description so
  review can focus on inferred behaviour and invented abstractions.
