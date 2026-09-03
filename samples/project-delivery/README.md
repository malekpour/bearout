# Sample: project-delivery

## Purpose

A fictional project delivery model: a project, participants with roles,
work packages, milestones, deliverables with acceptance criteria, and
budget allocations. It is the shape of any plan in which numbers, dates,
and roles spread across many files must agree.

## Data classification

Fictional. People, project, and figures are invented; budget amounts are
abstract budget units, not money.

## Capabilities demonstrated

- **Cross-resource arithmetic.** Allocations must sum to the project
  budget exactly; integer budget units keep the arithmetic exact.
- **Contiguous ordering.** Work-package and milestone sequences must be
  1, 2, 3, … per project, through one shared helper loaded with `load()`.
- **Chronological consistency.** Milestones fall due in sequence order and
  within the project's dates; dates compare as ISO-8601 strings.
- **Role constraints.** Exactly one lead per project, the project's `lead`
  has the lead role, work packages are owned by engineers or the lead, at
  least one reviewer participates.
- **Deliverable-to-milestone relationships.** A deliverable's milestone
  belongs to the same project, and every milestone carries a deliverable,
  found through the reverse index.
- **Documents from the graph.** A delivery plan and a CSV schedule per
  project, with acceptance criteria pulled from the deliverables' sections.

## Resource model

| Kind | Key fields | Relations |
| --- | --- | --- |
| `project` | `title`, `budget`, `starts`, `ends`, `## Scope` | `lead` → participant |
| `participant` | `name`, `role` | `project` |
| `work-package` | `title`, `sequence` | `project`, `owner` → participant |
| `milestone` | `title`, `sequence`, `due` | `project`, `work_package` |
| `deliverable` | `title`, `acceptance_days`, `## Acceptance criteria` | `project`, `milestone` |
| `allocation` | `amount` | `project`, `work_package` |

## Generated artifacts

- `generated/project-lantern-plan.md`
- `generated/project-lantern-schedule.csv`

## Try breaking it

- Change `allocation-pipeline` to `amount = 35`: B015 `budget-total` on
  the project.
- Set `milestone-pipeline-live` `due = "2026-11-01"`: B015 `chronology`.
- Set `milestone-handover` `due = "2027-04-30"`: B015 `chronology`, outside
  the project dates.
- Change `participant-ivo-brandt`'s role to `lead`: B015 `one-lead`.
- Set `work-package-design`'s owner to `participant-teo-alder`: B015
  `owner-role`.
- Point `deliverable-pipeline` at `milestone-design-review`: B015
  `milestone-empty` on `milestone-pipeline-live`.
- Delete `allocation-design.md`: B015 `budget-total` and B016 `unfunded`.

## Sample omissions

No date arithmetic, no calendar or capacity model, no change history, no
money or currency.

## Engine gaps

- Dates compare lexically. Adding an acceptance window to a due date would
  need date arithmetic, which neither Starlark's standard library nor
  Bearout provides.
