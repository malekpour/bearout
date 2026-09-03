# Sample: linked-notes

## Purpose

The smallest complete Bearout project. Read it before any other sample: it
shows the static bootstrap, one Starlark entry module, a shape, a typed
reference, a Markdown link, a header-only TOML resource, and one repository
rule that needs logic.

## Data classification

Synthetic. The notes describe the sample itself.

## Capabilities demonstrated

- `bearout.toml` grants the resource root and the rules root; there are no
  templates and no outputs.
- `bearout.star` registers two schemas. The note schema has a shape and a
  validator loaded through contained `load()`; the tag schema has a shape
  only.
- `note.schema.toml` declares `next` as a typed relation to another note
  and `tags` as a typed relation to tags. The kernel resolves both and
  rejects a target of the wrong kind.
- `x-bearout.sections` requires a `Summary` heading; the kernel checks it.
- `tag-start-here.toml` is a header-only resource: front matter without a
  body.
- The Markdown links, including the `#summary` fragment, are resolved
  against the other note's headings.
- `validate_note` reports an empty Summary with a line number and a rule
  code, `empty-summary`.

## Resource model

| Schema | File | Fields |
| --- | --- | --- |
| `example/linked-notes/note@1` | `content/note-*.md` | `title`, `next` → note, `tags` → tag |
| `example/linked-notes/tag@1` | `content/tag-*.toml` | `label` |

Identifiers begin with their kind and equal the file stem.

## Generated artifacts

None. This sample has no generators.

## Try breaking it

- Change `next = "note-reading-order"` to `next = "tag-start-here"`: the
  kernel reports B010, a relation of the wrong kind.
- Rename `## Summary` to `## Overview`: B006, missing section.
- Add `titel = "x"` to a note: B005, an unknown field.
- Delete the text under `## Summary`: B015 from `validate_note`, with the
  heading's line and the rule code.
- Change the link to `note-welcome.md#intro`: B011, unresolved anchor.

## Sample omissions

No generation, no project-level check, no fragments. Those appear in
`decision-records`.

## Engine gaps

None for this sample.
