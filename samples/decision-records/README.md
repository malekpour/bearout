# Sample: decision-records

## Purpose

A decision log with a lifecycle and citable rulings. Each record is
proposed, then accepted, rejected, or superseded. An accepted record
publishes rulings as fragments under headings named by the ruling
identifier, so prose anywhere can link to one ruling and the kernel resolves
the link.

## Data classification

Synthetic. The decisions are about this sample's own conventions.

## Capabilities demonstrated

- **Shape-first validation.** `decision.schema.toml` owns types,
  enumerations, the date format, unknown-field rejection, the required
  `Context` section, both typed relations, and the ruling fragment shape.
- **Fragments with identifiers.** Rulings are fenced TOML blocks whose ids
  join the project namespace; a duplicate ruling id anywhere is B008.
- **Shared helpers through contained `load()`.** `rules/lib/records.star`
  is loaded by the validator, both checks, and the generator.
- **Per-record logic.** `validate_decision` enforces ruling sequence, the
  heading-per-ruling convention, and status-dependent fields, each with a
  rule code and, where it applies, a line.
- **Log-wide checks.** `supersession-is-reciprocal` and
  `numbering-is-contiguous` run over the whole graph and attach findings to
  the record at fault.
- **Generation with provenance.** `decision-index` renders
  `generated/decision-index.md`; the kernel stamps the SPDX and provenance
  header and records ownership in `bearout-state.toml`.
- **Policy-defined immutability.** With `--baseline <rev>`,
  `protected-records-are-immutable` in `rules/immutability.star` compares
  the candidate with the historical log that Bearout projects through this
  policy: a resolved record (accepted, rejected, or superseded) may not be
  deleted, may not change its date, rulings, or Decision section, and may
  change status only from accepted to superseded; its title, relations,
  and Context may be corrected; moving it is a warning; new records are
  free. This README is a selected document, and removing it from the
  selection is reported against the baseline. Without a baseline the check
  is inactive. Which records are protected and what may change is this
  repository's decision, not the kernel's.

## Resource model

| Schema | Fields | Fragments |
| --- | --- | --- |
| `example/decision-records/decision@1` | `title`, `status`, `date`, `supersedes` → decision, `superseded_by` → decision | `ruling`: `id`, `text` |

Records: `decision-0001` accepted, `decision-0002` superseded,
`decision-0003` proposed, `decision-0004` accepted and superseding,
`decision-0005` rejected.

## Generated artifacts

- `generated/decision-index.md`: table of records and the list of rulings
  with links to their headings.

## Try breaking it

- Renumber `decision-0001-ruling-02` to `-03`: B015 `ruling-sequence`.
- Rename the `### decision-0001-ruling-02` heading: B015 `ruling-heading`.
- Point `superseded_by` in `decision-0002` at `decision-0001`: two B015
  findings from `supersession-is-reciprocal`, one on each record.
- Delete `decision-0003.md`: B015 `numbering` on `decision-0004`.
- Change a `status` to `done`: B005 from the shape; the validator never runs
  for that record.
- Edit `generated/decision-index.md` by hand: `bearout generate --check`
  reports B020 stale.
- Commit the sample, then change a ruling's `text` in `decision-0001` and
  run `bearout check --baseline HEAD`: B015 `protected-rulings`. Delete
  `decision-0005.md` instead: B015 `protected-record-deleted`, reported on
  the baseline side as `baseline:records/decision-0005.md`. Change only a
  title: clean.

## Sample omissions

No warnings, no header-only resources, no multi-output generators.

## Engine gaps

- **Bare citations.** Only Markdown links are resolved. A ruling id written
  as plain text is not checked.
- **No inferred baseline.** Immutability is checked only against a
  revision named explicitly; nothing compares against a parent or a merge
  base on its own.
