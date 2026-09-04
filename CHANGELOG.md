# Changelog

All notable changes to Bearout will be documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and releases follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Added

- Experimental Git-backed sources. `bearout check` and
  `bearout generate --check` accept `--index`, which reads the Git index as
  captured at the start of the run (what a commit would record: staged
  additions and modifications present; unstaged edits, untracked files,
  staged deletions, and intent-to-add entries absent; an unmerged index
  fatal), or `--revision <rev>`, which reads one commit, tag, branch, or
  tree, resolved exactly once. Every input of the run, from the bootstrap
  to the generated outputs that check mode verifies, comes from the
  selected tree; a working-directory file never satisfies a lookup in a
  Git-backed run. Projects below the repository root and linked worktrees
  are supported; symbolic links resolve only inside the tree; submodules
  are never entered. Requires the `git` executable. In the library, the
  source is `Options::source` (`Source::WorkingDirectory`, the default,
  `Source::Index`, or `Source::Revision`), and a Git-backed report records
  the source in `Report::source`, serialized as an experimental `source`
  object in JSON with a deterministic `digest` of the captured entries.
  Git runs with a fixed environment: repository, object, and
  configuration redirection variables are dropped, replacement objects and
  lazy fetching are disabled, the index is captured from one private copy,
  and `GIT_INDEX_FILE` is honoured only for a regular file directly inside
  the repository's own Git directory.

- Schema-less Markdown documents. An explicit, read-only `[documents]`
  grant (`roots` walked recursively for `*.md`, `files` named one by one)
  selects ordinary Markdown files, which are parsed with the resource body
  model but get no schema, identifier, or shape. Links and images of
  resources and documents are checked together: relative and `/`-rooted
  targets, query strings, percent escapes, same-document and
  cross-document fragments against GFM heading anchors and explicit
  `<a id>`/`<a name>` anchors, existing files and directories as link
  targets, existing files as image targets. A fragment on a Markdown file
  that is neither a resource nor a selected document is reported instead
  of assumed valid. New limits `limits.documents` and
  `limits.document_bytes`; new code B022 for a document that cannot be
  read; the report and its JSON carry a `documents` count. Policy sees
  `project["documents"]` and may report findings with `path=` against a
  document; resource views gain `anchors`, `images`, and `links[].text`.
  The `document-references` sample shows the slice.

- Experimental candidate/baseline comparison. `--baseline <REV>` (library:
  `Options::baseline`) names one exact Git revision of the same repository
  to compare the candidate against; nothing is inferred. It composes with
  every candidate source and with `generate --check`; writing generation
  still needs a working-directory candidate. The baseline is projected
  through the candidate's policy: its own `bearout.toml` selects which
  paths it classified as resources and documents, the candidate's limits
  bound it, and the candidate's schemas and shapes validate it; no
  baseline Starlark, generator, or template ever runs, and a revision
  without a `bearout.toml` is an empty historical project. Baseline
  problems keep their codes and carry a structured `side` (`baseline` in
  JSON, a `baseline:` prefix in text). Policy sees
  `project["comparison"]` with the historical `baseline` view and
  deterministic `changes` over the contract surface (bootstrap,
  resources, documents; added, removed, modified; no rename heuristics),
  and `error()`/`warning()` accept `side="baseline"` to target a
  history-only resource or document. The report carries the resolved
  baseline identity as `baseline`. What is immutable is entirely the
  repository policy's decision; the `decision-records` sample shows one.

- Experimental repository hygiene and formatting. An explicit `[hygiene]`
  grant selects files (`scope = "repository"` for every file as Git knows
  it, `scope = "declared"` for listed roots and files, refined by
  `exclude`, `binary`, and `text`), bounded by `limits.files`,
  `limits.file_bytes`, and `limits.hygiene_bytes` (the total read for
  hygiene in one run) and confined to the candidate. Native text hygiene
  enforces `charset`, `end_of_line`, `insert_final_newline`, and
  `trim_trailing_whitespace` from the `.editorconfig` files of the
  selected tree (codes B023 to B028). `[[formatters]]` declares
  repository-pinned programs run through a stdin/stdout byte-transform
  protocol with `{path}` substitution, support files from the selected
  tree, bounded streams, and a timeout, only under `--allow-formatters`
  (codes B029 and B030). `bearout format` rewrites selected working-tree
  files safely (B031). The report counts selected `files` and lists
  `formatted` paths.

- Experimental contract fixtures. An explicit `[fixtures] files` grant
  names TOML fixture files whose `[[cases]]` derive a virtual candidate
  from the selected source through a read-only overlay (`write` with
  inline `content` or a `payload` file, `delete`, `move`), optionally
  compare it with the unmodified source (`baseline = true`), and expect
  `clean`, `diagnostics`, or `fatal`. Expected diagnostics are matched
  structurally (`code`, `severity`, `path`, `line`, `side`, `rule`,
  `message`) as a multiset under `match = "exact"` (default) or
  `"contains"`. `bearout test [PATH]` (library: `bearout::test`) runs the
  suite from the working directory, `--index`, or `--revision`, never
  writes, formats, or delivers, refuses `--baseline`, keeps formatters
  behind `--allow-formatters`, and exits 0 when every case passed, 1 on an
  assertion failure, 2 when the suite cannot run. The test report
  (`TestReport`, JSON for every outcome) is a surface distinct from the
  contract report; assertion failures carry no B-series code. New limits
  `limits.fixture_cases`, `limits.fixture_mutations`, and
  `limits.fixture_bytes`. `check`, `generate`, and `format` never execute
  fixtures. The `decision-records` sample declares a suite.

- Experimental repository history and commit policy. `bearout history
  range [PATH] [--base REV] [--head REV]` checks the commits reachable
  from the head (default `HEAD`) but not from the base, both resolved
  once and recorded with full identities, merges included, oldest first
  in a deterministic topological order with the object identity as the
  tie-breaker; `bearout history message [PATH] --file FILE` is the
  commit-msg hook path over the exact message file (a regular file inside
  the repository's Git directory), the author Git would record, `HEAD`
  and any `MERGE_HEAD`, and the staged changes of the captured index.
  Policy comes from the resolved head's tree or the captured index, never
  the working tree, and only history checks run. The entry module
  registers `history_check(name, function)`; the check receives an
  immutable history view with raw identities (no `.mailmap`; a pending
  commit's author has no timestamp or timezone, because Git would only
  invent the current clock), byte-exact messages whose logical lines end
  at CRLF, LF, or a lone CR, ordered parents (an unborn branch only when
  Git proves it, and a `MERGE_HEAD` that must be a regular file naming
  existing commits), and changes against the first parent (or the empty
  tree for a root) with exact modes and object identities and no rename
  detection. `error()` and `warning()` accept `commit=` for a key
  of the view or nothing for a range-wide finding; new codes B032 and
  B033 appear only in the distinct history report, where any finding
  exits 1 and an unresolvable revision, a missing object, or a shallow
  boundary inside the range is fatal (exit 2). New limits
  `limits.history_commits`, `limits.history_changes`,
  `limits.history_commit_bytes`, and `limits.history_bytes`. Fixture
  cases may supply a synthetic pending message with `[cases.history]`,
  bounded by both history byte limits, with author time only as explicit
  synthetic facts, and expect findings by `commit`. Conventional Commits
  and DCO sign-offs are policies a repository supplies; the
  `commit-policy` sample shows one.
  In the library: `bearout::history`, `HistoryMode`, `HistoryReport`,
  `HistoryDiagnostic`, and `HistoryTarget`.

### Changed

- Markdown reference checking moved out of the identifier graph; explicit
  `<a id>`/`<a name>` anchors now satisfy fragments in resources too, and
  images are checked.
- The kernel reads every source through one read-only tree interface;
  writes go through a separate working-directory delivery capability, so
  `generate --check` needs no write access and writing generation against
  a Git-backed source is a fatal outcome (exit 2).
- Shape files are no longer read through a symbolic link (B001), which
  `docs/design.md` already documented for rules and shapes; rule modules
  behaved this way before.
- `Options` gained the `source` field; code constructing it by struct
  literal should use `..Options::default()`.
- `cargo run` in the repository runs the `bearout` binary by default; the
  feature-gated test fixture formatter must be named with `--bin`.
- `CaseResult::unexpected` holds `Reported` values, a contract or a
  history diagnostic serialized exactly as the diagnostic itself, so a
  history fixture case can list what it did not expect; `Expectation`
  gained the `commit` field and `Code::ALL` grew to 33 codes.

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
