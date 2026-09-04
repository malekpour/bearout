# Sample: document-references

## Purpose

Shows schema-less Markdown documents next to resources: `bearout.toml`
selects two guides and this README as documents, the kernel resolves every
link, image, heading anchor, and explicit anchor across resources and
documents, and one repository rule judges link text and alt text.

## Data classification

Synthetic. The guides describe the sample itself.

## Capabilities demonstrated

- `[documents]` grants `guides` recursively and `README.md` by name; the
  grant is read-only and explicit, nothing else is discovered.
- Documents have no envelope, schema, or identifier. They are parsed with
  the same Comrak model as resource bodies.
- Links resolve from a document into another document, into a resource,
  and root-relatively (`/README.md#purpose`); fragments resolve against
  GFM heading anchors, duplicate-heading suffixes (`#install-1`), and an
  explicit `<a id="options">` anchor. See [getting
  started](guides/getting-started.md#install).
- The image in the [reference](guides/reference.md#options) must name an
  existing file.
- `check_document_links` reports vague link text and empty alt text
  against the document path and line, with rule codes.

## Resource model

| Schema | File | Fields |
| --- | --- | --- |
| `example/document-references/topic@1` | `topics/topic-*.md` | `title` |

Documents: `guides/**/*.md` and `README.md`.

## Generated artifacts

None. This sample has no generators.

## Try breaking it

- Change `#options` to `#option` in `guides/getting-started.md`: B011, an
  anchor the reference does not define.
- Delete `guides/figures/flow.svg`: B011, a missing image.
- Link to `guides/reference.md#options` from a Markdown file that is not
  selected in `[documents]`: nothing, because unselected files are not
  checked; link from a selected one to an unselected `notes.md#x`: B011,
  an anchor Bearout cannot verify.
- Change `[reference](reference.md#options)` to `[here](reference.md#options)`:
  B015 from `check_document_links` with the code `descriptive-link-text`.
- Remove the image's alt text: B015 with the code `image-alt-text`.

## Sample omissions

No generation and no typed relations between documents; documents have no
identifiers by design.

## Engine gaps

None for this sample.
