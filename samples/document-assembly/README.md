# Sample: document-assembly

## Purpose

A fictional contributor handbook assembled from versioned, reusable
sections and a glossary. Sections are approved, superseded, or retired
independently; a handbook names the sections it includes in order, and the
generator assembles the document with placeholders substituted and a
glossary limited to the terms the included sections link to.

## Data classification

Fictional. The project, team, and policies are invented.

## Capabilities demonstrated

- **Relations to fragments.** A section's `uses_terms` targets glossary
  term fragments through the kind `example/document-assembly/glossary@1#term`;
  a section id in that field is B010.
- **Terminology through explicit references.** The validator compares the
  glossary anchors a section's text links to with its declared
  `uses_terms`, in both directions, using the kernel's link view rather
  than substring matching.
- **Placeholder validation before rendering.** Only `{Project}` and
  `{Team}` may appear in section text; any other `{...}` token is B015
  before a generator runs.
- **Versioning.** A superseded section must be retired and share its
  successor's category; a handbook assembles only approved, current
  sections, one per category, with every required category present.
- **Assembly.** Sections are numbered in the handbook's order, placeholders
  are substituted from handbook fields, and the glossary JSON carries only
  the selected terms.

## Resource model

| Kind | Key fields | Relations |
| --- | --- | --- |
| `section` | `title`, `category`, `version`, `status`, `## Text` | `supersedes` → section, `uses_terms` → glossary term fragment |
| `glossary` | `title`, term fragments (`id`, `term`, `text`) | none |
| `handbook` | `title`, `project_name`, `team_name`, `status`, `required_categories`, `## Introduction` | `glossary` → glossary, `sections` → section |

## Generated artifacts

- `generated/handbook-contributor.md`
- `generated/handbook-contributor-glossary.json`

## Try breaking it

- Add `{Company}` to a section's text: B015 `unknown-placeholder`.
- Remove `"glossary-core-term-maintainer"` from `section-review-v2`'s
  `uses_terms`: B015 `term-undeclared`.
- Add `"glossary-core-term-changelog"` to `section-conduct`'s
  `uses_terms`: B015 `term-unlinked`.
- Set `section-review-v1` to `status = "approved"`: B015
  `superseded-retired`.
- Replace `section-review-v2` with `section-review-v1` in the handbook: two
  B015 findings, `section-approved` and `section-current`.
- Add `section-triage` to the handbook: B015 `section-approved` and
  `category-unique`.
- Set `uses_terms = ["section-conduct"]`: B010, wrong kind.

## Sample omissions

No approval workflow, no dates, no per-section authorship, no rendering of
the glossary as its own document.

## Engine gaps

- Which handbook versions were published with a now-retired section is a
  question about history; Bearout reads one tree.
