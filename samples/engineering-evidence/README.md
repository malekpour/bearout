# Sample: engineering-evidence

## Purpose

A hardware-shaped evidence graph for a fictional modular controller
platform: engineering questions, decisions with a stated basis, evidence
sources, measurement records, interfaces, and modules. It shows how a
project can refuse free-text numbers and require every figure to trace to a
record, and every record to a source.

## Data classification

Synthetic. Every measurement is a fixture value invented for engine
development. Nothing here is a measurement of a real device, a datasheet
figure, or a manufacturer claim, and the shapes make each record say so.

## Capabilities demonstrated

- **Typed evidence.** A module's `figures` are relations to measurement
  records; a measurement's `source` is a relation to a source resource;
  `fixture = true` and a `note` beginning "Synthetic fixture value" are
  mandatory through the shape.
- **Analysis versus measurement.** A decision states its `basis`. JSON
  Schema `if`/`then` requires `evidence` when the basis is measurement; a
  warning check flags an analysis-based decision that cites measurements.
- **Question lifecycle.** Open, blocked, and closed questions, with the
  validator tying status to `blocked_by`, `closed_by`, and the `Why it is
  open` section.
- **Reciprocal closure.** `closes` on a decision and `closed_by` on a
  question must agree; nothing may be blocked by a closed question.
- **Rulings as fragments** under headings named by their identifier.
- **Registers.** A question register, an evidence register that lists each
  measurement's source and consumers through the reverse index, and a CSV.

## Resource model

| Kind | Key fields | Relations |
| --- | --- | --- |
| `question` | `area`, `status` | `blocked_by` → question, `closed_by` → decision |
| `decision` | `status`, `basis`, `date`, ruling fragments | `closes` → question, `evidence` → measurement |
| `source` | `kind` (analysis-note, synthetic-fixture, standard-reference), `locator` | none |
| `measurement` | `quantity`, `value`, `unit`, `method`, `fixture`, `note` | `source` → source |
| `interface` | nested `signals[]` | none |
| `module` | `kind` | `interfaces` → interface, `figures` → measurement |

## Generated artifacts

- `generated/question-register.md`
- `generated/evidence-register.md`
- `generated/measurements.csv`

## Try breaking it

- Set `note = "Measured on the bench"` on a measurement: B005, the note
  must begin with "Synthetic fixture value".
- Set `fixture = false`: B005.
- Remove `evidence` from `decision-0002`: B005, required when the basis is
  measurement.
- Add `evidence = ["measurement-cycle-period"]` to `decision-0001`: B016
  `basis-mismatch`, a warning.
- Change `closed_by` on `question-0004` to `decision-0001`: two B015
  `closure-reciprocal` findings.
- Set `figures = ["source-fixture-bench"]` on a module: B010, wrong kind.
- Change `question-0002`'s `blocked_by` to `question-0003`: B015
  `blocked-by-closed`.

## Sample omissions

No units conversion, no tolerance or uncertainty fields, no history of
questions reopening.

## Engine gaps

- Units are an enumeration in the shape. Bearout has no unit model and does
  not check that a voltage is given in volts; the sample cannot express
  dimensional consistency without listing every valid pairing.
