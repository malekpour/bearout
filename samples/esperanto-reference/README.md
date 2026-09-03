# Sample: esperanto-reference

## Purpose

A small, sourced Esperanto reference: chapters, a subset of the sixteen
grammar rules of the Fundamento, the grammatical endings as morphemes, a
glossary of words attested in the Fundamento's exercises, and example
sentences quoted from those exercises. It exercises Unicode content and
headings, ASCII-portable identifiers, sourced facts, and repository-defined
collation.

## Data classification

Sourced snapshot. Rules are paraphrased from the English grammar of the
Fundamento de Esperanto as published by the Akademio de Esperanto; example
sentences are quoted verbatim from the Ekzercaro with their section
numbers; both pages were retrieved on 2026-09-03 and are recorded as source
resources with URL, publisher, and locator. English glosses of terms are
by the sample author. Claims that could not be verified against the fetched
pages were left out. Rights basis: the quoted and paraphrased text is the
1905 Fundamento, whose author died in 1917; the repository's `NOTICE.md`
states the project's good-faith reading of that and its limits. Bearout
claims no authorship of it.

## Capabilities demonstrated

- **Unicode headings and anchors.** Rules carry `## Regulo` and `## Rule`
  sections; chapter titles such as "Ĝeneralaj reguloj" produce GFM anchors
  with Esperanto letters, and links to them are resolved.
- **ASCII identifiers.** Ids use the x-system (`term-cxielo`,
  `example-sxi-devis`) because the kernel's identifier grammar is ASCII.
- **Sourced facts.** Every rule and example names a source resource and a
  locator (rule number or Ekzercaro section); a warning check flags any
  source outside the Akademio's site.
- **Rule-to-example relationships.** Examples name the rules they
  illustrate; a warning check reports a rule no example illustrates.
- **Repository-defined collation.** `rules/lib/eo.star` defines the
  Esperanto alphabet order and sorts the morpheme index and glossary with
  it; Bearout has no locale support.
- **Generated Markdown and JSON.** A grammar reference, a morpheme index, a
  glossary, and an examples corpus.

## Resource model

| Kind | Key fields | Relations |
| --- | --- | --- |
| `chapter` | `title`, `sequence`, `## Celo`, `## Purpose` | none |
| `rule` | `title`, `number`, `locator`, `## Regulo`, `## Rule` | `chapter`, `source` |
| `morpheme` | `form`, `kind`, `meaning` | `rule` |
| `term` | `esperanto`, `english`, `part_of_speech` | `morphemes` → morpheme, `attested_in` → example |
| `example` | `esperanto`, `english`, `locator` | `rules` → rule, `source` |
| `source` | `title`, `publisher`, `url`, `retrieved`, `## Summary` | none |

## Generated artifacts

- `generated/grammar-reference.md`
- `generated/morpheme-index.md`
- `generated/glossary.json`
- `generated/examples.json`

## Try breaking it

- Change a source `url` to `https://example.org/`: B016
  `source-authority` on every rule citing it.
- Delete `example-lernolibron.md`: B016 `rule-coverage` on
  `rule-11-kunmetado`.
- Set `attested_in = ["rule-02-substantivo"]` on a term: B010, wrong
  kind.
- Change a chapter `sequence` to 5: B015 `sequence`.
- Rename `## Regulo` to `## Regulo 2` in a rule: B006, missing section.
- Link to `rule-02-substantivo.md#regulo-2`: B011, unresolved anchor.

## Sample omissions

Prefixes and suffixes, correlatives, numerals, participles, pronunciation
of individual letters, and most of the sixteen rules; only what the fetched
sources support is included.

## Engine gaps

- Collation is repository policy. Bearout cannot sort generated output
  itself, so a template can only present the order a generator computed.
- Terms are not proven to occur in the examples they cite; that would need
  morphological analysis the sample does not attempt.
