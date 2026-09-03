# Sample: multilateral-records

## Purpose

A larger conditional and temporal graph: one multilateral instrument, its
articles, original and acceding parties with lifecycles, and related
instruments adopted under it. The compact's entry-into-force date is not a
free field; the checks compute it from the parties on file and require the
record to agree.

## Data classification

Fictional. The Aurora Research Compact, its polities, places, dates, and
depositary are invented. No real state, institution, treaty, or depositary
record is described, and every resource says so.

## Capabilities demonstrated

- **Computed entry into force.** Under the all-signatories rule the compact
  entered into force on the day of the latest deposit by an original
  signatory; `in_force` must equal it.
- **`allOf` of `if`/`then` in the shape.** A signatory must have signed and
  ratifies; an acceding party accedes or succeeds; a consultative party
  records when it attained that status. No script is involved.
- **Date consistency across resources.** Signatories signed on the
  compact's date, nobody deposits before signing, acceding parties deposit
  after signature, consultative status follows the deposit, instruments
  are adopted after the parent entered into force and in force after
  adoption, articles are numbered contiguously.
- **Three outputs from one context.** An overview with article summaries, a
  party status table in deposit order, and a CSV snapshot.

## Resource model

| Kind | Key fields | Relations |
| --- | --- | --- |
| `compact` | `signed`, `place`, `in_force`, `depositary`, `languages`, `entry_into_force`, `original_signatories`, `## Summary` | none |
| `article` | `number`, `title`, `topics`, `## Summary` | `compact` |
| `party` | `code`, `status`, `signed`, `deposited`, `instrument`, `consultative`, `consultative_since`, `note` | `compact` |
| `instrument` | `kind`, `adopted`, `place`, `in_force`, `## Summary` | `parent` → compact |

## Generated artifacts

- `generated/compact-aurora-overview.md`
- `generated/compact-aurora-parties.md`
- `generated/compact-aurora-parties.csv`

## Try breaking it

- Set `party-ilvaren-reach` `deposited = "2032-07-01"`: B015
  `entry-into-force` on the compact.
- Set `party-orvelune` `signed = "2031-03-13"`: B015 `signed-date`.
- Set `party-sarralind-union` `consultative_since = "2033-01-01"`: B015
  `consultative-date`.
- Set `party-veyra` `instrument = "ratification"`: B005, bound to the
  status by the shape.
- Remove `consultative_since` from `party-meridia`: B005, required when
  consultative.
- Set `instrument-data-protocol` `in_force = "2033-01-01"`: B015
  `instrument-date`.
- Renumber `article-07` to 8: B015 `article-number`.

## Sample omissions

Threshold entry-into-force rules, withdrawal and succession as events,
reservations and declarations, and per-party participation in the
instruments.

## Engine gaps

- Withdrawal and succession are events over time. A party view has one
  status; a history of status changes would need either fragments per event
  or a history-aware phase Bearout does not have.
