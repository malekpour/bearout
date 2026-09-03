# Changelog

All notable changes to Bearout will be documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and releases follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

Nothing yet.

## 0.1.0 - 2026-09-03

Initial release. Everything below is experimental; the bootstrap, the
resource envelope, the `x-bearout` vocabulary, the Starlark ABI (version 0),
diagnostic codes, and generated outputs are contract surfaces without a
compatibility promise yet.

### Added

- A static bootstrap, `bearout.toml`, that names the Starlark entry module
  and grants resource, rules, templates, and output roots plus resource
  limits. Roots must be disjoint.
- Repository policy in Starlark: an entry module registers schemas, checks,
  and generators; a contained `load()` resolves beneath the rules root and
  rejects escapes, symlinks, and cycles; every module is linted and
  statically typechecked; every evaluation runs under tick, heap, and
  call-stack limits with cancellation.
- A narrow ABI: frozen resource and project views in, typed host values
  out (`error()`, `warning()`, `output()`), with every field validated.
- The resource envelope parsed with `toml_edit` over an exact byte range,
  Markdown bodies kept byte-for-byte, header-only `.toml` resources, and
  native TOML dates normalized to their TOML text.
- Markdown structure through Comrak: sections with GFM anchors, fenced
  blocks, typed fragments, and links resolved against files and anchors.
- JSON Schema 2020-12 shapes authored in TOML with a validated `x-bearout`
  vocabulary for typed relations, required sections, and fragment kinds.
- A phased pipeline in which invalid resources never reach policy and
  generation runs only on an error-free project.
- Confined, staged generation through a `cap-std` capability with atomic
  per-file delivery, best-effort restoration, a BLAKE3 provenance state
  manifest, orphan handling, and `bearout generate --check`.
- A JSON report for every outcome, exit codes 0, 1, and 2, and the
  diagnostic catalog in `docs/diagnostics.md`.
- Eight samples: `linked-notes`, `decision-records`, `esperanto-reference`,
  `formula-language`, `engineering-evidence`, `project-delivery`,
  `document-assembly`, and `multilateral-records`.

### Fixed

- Output delivery writes through an exclusively created, uniquely named
  temporary file (`cap-tempfile`) instead of a predictable sibling name
  that a pre-existing symbolic link could redirect; a symbolic link is never
  followed or installed, and temporary files are removed on failure.
- Percent-encoded Markdown links containing non-ASCII text no longer panic;
  decoding works on bytes, and decoded paths are revalidated (B011).
- `bearout-state.toml` is parsed strictly; an invalid manifest is B020 and
  stops the run before any file is touched.
- An existing output the manifest does not own is never overwritten, even
  when its bytes already match (B021); delivery is journaled and restored
  on failure, and the manifest is never written after a failed step.
- `Report.outputs` lists files only when generation succeeded.
- Discovery reports non-UTF-8 or non-portable file names instead of skipping
  them; logical paths reject backslashes instead of rewriting them.
- Template rendering is bounded by `limits.template_fuel` and
  `limits.output_bytes`; the shape suffix `.schema.toml` and the SPDX form
  of `[outputs] license` are enforced.
- A `+++` line inside a TOML multi-line string no longer ends the front
  matter early; the closing fence is the first one at which the header
  parses.
- A number that fits neither an integer nor a float is a diagnostic when
  building script views or template contexts, not a substituted zero.
- The whitespace check covers untracked files, so it validates the
  candidate tree before the first commit.

### Removed

- The Rhai runtime and every `.rhai` file, replaced by Starlark.
- The earlier samples with real-world hardware figures, a purported executed
  legal agreement, and real treaty records, replaced by synthetic or
  fictional data.
