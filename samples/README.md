# Samples

Each directory is a complete Bearout project: a static `bearout.toml`, a
Starlark entry module, shapes and rules beneath `rules/`, resources, and
where generation is enabled, templates and committed outputs with a
`bearout-state.toml`. Every sample is checked and its outputs verified by
`cargo test` (`tests/samples.rs`), which is the one authoritative sample
run in `mise run check`.

The samples are repository test and reference material. They are
deliberately excluded from the crates.io package (`Cargo.toml` `include`),
so a published crate never carries them or their generated outputs.

Read them in this order.

| Sample | Classification | Models | Capabilities |
| --- | --- | --- | --- |
| [`linked-notes`](linked-notes/) | synthetic | Three linked notes | Bootstrap, one validator, typed reference, link and anchor resolution, header-only TOML resource |
| [`decision-records`](decision-records/) | synthetic | A decision log with citable rulings | Fragments, shared helpers via `load()`, project checks, a generator with provenance, policy-defined immutability against a baseline |
| [`esperanto-reference`](esperanto-reference/) | sourced snapshot | A small Esperanto grammar reference | Unicode headings and anchors, sourced facts, repository-defined sorting, Markdown and JSON outputs |
| [`formula-language`](formula-language/) | synthetic | Formulo, a spreadsheet-expression language | Grammar as a graph, a generated Rust lexer, parser, AST, and conformance tests that compile and pass |
| [`engineering-evidence`](engineering-evidence/) | synthetic | A hardware-shaped evidence graph | Typed evidence, analysis versus measurement, reciprocal closure, generated registers |
| [`project-delivery`](project-delivery/) | fictional | A project delivery model | Cross-resource arithmetic, ordering and chronology, role constraints, plan and CSV outputs |
| [`document-assembly`](document-assembly/) | fictional | A contributor handbook from versioned sections | Supersession, relations to glossary fragments, placeholder validation, assembled document |
| [`document-references`](document-references/) | synthetic | Guides beside a topic resource | Schema-less documents, cross-document links, heading and explicit anchors, images, a policy rule over link text and alt text |
| [`multilateral-records`](multilateral-records/) | fictional | The Aurora Research Compact | Conditional shapes, computed entry into force, chronology, party status outputs |

## Conventions

- Identifiers and file names are lowercase kebab-case; the file stem equals
  the resource id; ids begin with their kind; fragment ids derive from
  their parent's id. The samples enforce these as policy; the kernel does
  not.
- Schema identifiers are `example/<sample>/<kind>@1` and belong to the
  sample, not to Bearout.
- Shapes are `*.schema.toml`; Starlark files are `*.star`; templates are
  `*.j2`; check names describe the invariant; generator names describe
  their output.
- Generated files live beneath a declared output root and carry SPDX and
  provenance headers where the format allows comments.
- Every README uses the same sections: Purpose, Data classification,
  Capabilities demonstrated, Resource model, Generated artifacts, Try
  breaking it, Sample omissions, Engine gaps.

## Adding a sample

Add a directory following the conventions above, list it in
`tests/samples.rs`, and add a row here. Grow it until the engine cannot
express what it needs, then grow the engine.
