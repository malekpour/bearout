# Bearout

Bearout is a deterministic repository contract engine. A repository keeps
prose and structured metadata together as resources; Bearout discovers
them, validates their shape, resolves their relationships, applies the
repository's own rules, and generates artifacts from the verified graph,
with provenance.

A *contract* here is a machine-checkable agreement about the resources in a
repository. It is not necessarily a legal contract.

> [!WARNING]
> Bearout is an experiment. Its bootstrap, resource envelope, shape
> vocabulary, Starlark ABI (version 0), diagnostic codes, and generated
> outputs are not stable yet. Do not build a compatibility promise on the
> current syntax.

## Two layers

- The **Rust kernel** owns discovery, parsing, graph construction,
  diagnostics, resource limits, path confinement, and filesystem writes.
- The **repository** owns every domain rule: schema identifiers and their
  JSON Schema shapes, validators, project checks, and generation plans in
  Starlark, and templates.

A schema identifier such as `example/decision-records/decision@1` belongs to
the repository that defines it. Nothing is registered with Bearout or
compiled into the binary. See [`docs/design.md`](docs/design.md) for the
responsibilities, phases, and boundaries.

## The bootstrap

`bearout.toml` is static and is the capability boundary of a project:

```toml
version = 1
entry = "bearout.star"

[resources]
roots = ["records"]

[rules]
root = "rules"          # `load()` resolves here; shapes live here

[templates]
root = "templates"

[outputs]
roots = ["generated"]   # the only places generation may write
license = "Apache-2.0"  # stamped into generated headers

[documents]             # optional; schema-less Markdown, read-only
roots = ["docs"]        # walked recursively for `*.md`
files = ["README.md"]   # named one by one

[fixtures]              # optional; contract fixture files for `bearout test`
files = ["contract-tests/log.test.toml"]

[limits]                # optional; see docs/design.md for which defaults are measured
ticks = 1000000
template_fuel = 2000000
```

Repository policy can register schemas, checks, and generators. It cannot
widen the roots the bootstrap grants. Resource, rules, templates, and
output roots are disjoint, none is the project root, and all filesystem
access goes through a capability opened on the project root. The document
grant is read-only and may overlap any of them.

## Repository policy

The entry module registers what the project uses:

```python
load("decision.star", "validate_decision")
load("log.star", "check_supersession_is_reciprocal")
load("decision-index.star", "plan_decision_index")

schema("example/decision-records/decision@1",
       shape = "decision.schema.toml", validate = validate_decision)
check("supersession-is-reciprocal", check_supersession_is_reciprocal)
generator("decision-index", plan_decision_index)
```

A validator receives one frozen resource view and returns findings; a check
receives the project view; a generator returns outputs:

```python
def validate_decision(resource):
    if resource["fields"]["status"] == "accepted" and not rulings(resource):
        return [error("an accepted record must carry a ruling", code = "rulings-required")]
    return []
```

`load()` resolves only beneath the rules root; escapes, symbolic links,
and cycles are rejected with the import chain. Every module is linted and
statically typechecked, and every evaluation runs under tick, heap, and
call-stack limits with cancellation. Scripts have no filesystem,
environment, network, clock, or random access. The full ABI is in
[`docs/starlark-abi.md`](docs/starlark-abi.md).

## Resources and shapes

A resource is Markdown with TOML front matter, or a header-only TOML file:

````markdown
+++
schema = "example/decision-records/decision@1"
id = "decision-0004"
title = "Records are numbered when merged"
status = "accepted"
date = "2026-09-02"
supersedes = ["decision-0002"]
+++

# Records are numbered when merged

## Context

### decision-0004-ruling-01

```toml bearout=ruling
id = "decision-0004-ruling-01"
text = "A record receives its sequence number when it is merged."
```
````

`schema`, `id`, and `refs` are the envelope keys the kernel owns. Every
other key is validated by the schema's shape, a JSON Schema 2020-12
document authored in TOML with a small `x-bearout` vocabulary:

```toml
"$schema" = "https://json-schema.org/draft/2020-12/schema"
type = "object"
additionalProperties = false
required = ["title", "status", "date"]

[properties.supersedes]
type = "array"
items = { type = "string" }
"x-bearout" = { ref = "example/decision-records/decision@1" }  # typed relation

["x-bearout"]
sections = ["Context"]                                          # required heading

["x-bearout".fragments.ruling]                                  # a fragment kind
type = "object"
required = ["id", "text"]
```

The vocabulary itself is validated: an unknown `x-bearout` key, a relation
on a non-string property, or a shape that declares an envelope key is an
error. Markdown bodies are parsed with Comrak; headings get GFM anchors,
fenced blocks tagged `bearout=<kind>` become typed fragments with
project-wide identifiers, and every relative link and `#anchor` is resolved.

## Schema-less documents

Ordinary Markdown files such as a README, governance notes, or design
documents carry no envelope and get no schema, identifier, or shape. An
explicit `[documents]` grant selects them: `roots` are walked recursively
for `*.md`, never following links or entering submodules, and `files` are
named one by one; nothing else is discovered, and a path that resource
discovery already claims is processed once, as a resource. Documents are
parsed with the same Comrak model as resource bodies: headings with GFM
anchors, explicit `<a id>` and `<a name>` anchors, links with their visible
text, and images with their alt text.

Every link and image of a resource or document is resolved against the
project tree: relative targets from the source's directory, `/`-prefixed
targets from the project root, `#fragment` within the source, and a
fragment on another Markdown file against that file's heading and explicit
anchors. A fragment on a Markdown file that is neither a resource nor a
selected document is reported rather than assumed valid; an image must
name an existing file; a broken reference is B011. Policy sees the
documents as `project["documents"]` and may report findings against a
document `path` and line. The [`document-references`](samples/document-references/)
sample shows the whole slice. Bearout assigns no meaning to any document
name or directory; which files matter is the repository's decision.

## Phases

bootstrap, discovery, parsing, structural validation, graph construction,
repository policy, generation planning, rendering, delivery. A resource that
fails parsing or structural validation is never passed to a validator, and
its identifiers still resolve so nothing cascades. Checks run only on an
error-free graph; generation runs only on an error-free project.

## Generation

Scripts never write files. A generator returns `output(template, path,
context)` entries; the kernel validates every path against the output
roots, renders every artifact into memory with MiniJinja in strict mode,
computes BLAKE3 digests, and only then delivers each file through an atomic
rename. `bearout-state.toml` records which outputs Bearout owns and their
provenance; `bearout generate --check` reports missing, stale, unowned,
orphaned, and re-owned outputs. A file the manifest does not own is never
overwritten, even when its bytes already match; orphans are removed only
when the manifest proves Bearout wrote them and they are unmodified. The
report's `outputs` list names delivered or verified files only when
generation succeeded. Third-party content is recorded in
[`NOTICE.md`](NOTICE.md).

## Commands

```sh
bearout check [path]                    # exit 0 clean, 1 findings, 2 fatal
bearout generate [path]                 # check, then deliver outputs
bearout generate --check [path]         # check, then verify committed outputs
bearout --format json check [path]      # one JSON report for every outcome
bearout check --index [path]            # check what a commit would record
bearout check --revision v1.2 [path]    # check one commit, tag, branch, or tree
bearout generate --check --index [path] # verify outputs as staged
bearout check --index --baseline HEAD   # and compare with an exact revision
bearout --allow-formatters check [path] # also run the declared formatters
bearout --allow-formatters format [path] # rewrite selected files in place
bearout test [path]                     # run the declared contract fixtures
bearout test --index [path]             # the suite, policy, and payloads as staged
bearout history range [path] --base REV # check the commits in REV..HEAD
bearout history message [path] --file .git/COMMIT_EDITMSG  # the commit-msg hook
```

Diagnostics use stable codes and forward-slash project-relative paths on
every platform; the catalog and its stability policy are in
[`docs/diagnostics.md`](docs/diagnostics.md).

## Sources

> [!WARNING]
> The Git-backed sources are experimental and require the `git`
> executable on `PATH`. Their flags, semantics, and report fields may
> change.

Without a selection, Bearout reads the live working directory through its
filesystem capability, as before; it makes no snapshot, so concurrent edits
are visible to a run. Two read-only sources read a Git tree instead, and
every input of the run comes from that tree and nothing else: the
bootstrap, the entry module and everything it loads, shapes, resources,
templates, `bearout-state.toml`, the generated outputs that
`generate --check` verifies, and the files that links resolve against. A
file that exists only in the working directory never satisfies a lookup in
a Git-backed run.

- `--index` reads the Git index of the repository that owns the project,
  from one private copy taken when the run starts: the tree a commit would
  record. Staged additions and modifications are present; unstaged
  modifications, untracked files, staged deletions, and intent-to-add
  entries are absent; a staged rename appears only at its destination. An
  unmerged index is a fatal outcome. In a partial-commit hook, Git's
  `GIT_INDEX_FILE` is honoured when it is a regular file directly inside
  the repository's own Git directory.
- `--revision <rev>` reads one commit, tag, branch, or tree object. The
  name is resolved exactly once, at the start; the resolved tree identity
  is recorded in the JSON report's `source` field and used for the whole
  run even if the branch moves meanwhile. An unknown name is a fatal
  outcome.

Either way the JSON report's `source` carries a deterministic `digest` of
the captured entries, equal for identical content from either source, so
a report can be tied to exactly what it examined. Git runs with a fixed
environment: variables that redirect the repository, its objects, or its
configuration are dropped, replacement objects and lazy fetching are
disabled, and nothing is fetched or written.

The project may sit below the repository root, including in a linked
worktree; only paths beneath the project are exposed, and a symbolic link
in a Git tree resolves only inside the tree it is read from. Submodules are
never entered. Blobs are read exactly as Git stores them, without
line-ending conversion or filters, and nothing is checked out.

Both sources are read-only: `check` and `generate --check` accept them,
`generate` without `--check` refuses them with exit code 2. Git support does
not make Bearout a security sandbox, and scripts do not learn which source
a run reads.

## Comparison

> [!WARNING]
> Comparison is experimental, like the Git-backed sources it builds on.

`--baseline <rev>` (library: `Options::baseline`) makes a run compare its
candidate, whatever source that is, with one exact Git revision of the
same repository. Nothing is inferred: no `HEAD`, parent, merge base, or
default branch. The name is resolved once, the candidate is checked as
usual, and the baseline is read-only historical evidence that is never
written and whose policy is never executed. The candidate's policy is the
only policy that runs: the baseline's own `bearout.toml` decides which
paths that revision classified as resources and documents, the
candidate's limits bound both sides, and the candidate's schemas and
shapes validate both. A revision without a `bearout.toml` is an empty
history; a baseline whose history the current policy cannot interpret is
reported on the baseline side, with `"side": "baseline"` in JSON and a
`baseline:` prefix in text, and fails the run.

Policy sees `project["comparison"]`, `None` without a baseline: the
historical `baseline` view with the same resource and document shapes as
the candidate, and `changes`, deterministic facts over the contract
surface (the bootstrap, the discovered resources, and the discovered
documents; not the whole repository) as `added`, `removed`, or `modified`
paths with each side's classification, digest, and size. A rename is a
removal plus an addition; resources pair through their ids. A check may
report against either side with `side="baseline"`, so a deleted record
can be named. What is immutable, when, and what may still change is the
repository's policy, never the kernel's; the
[`decision-records`](samples/decision-records/) sample shows one such
rule. The report carries the resolved baseline identity as `baseline`.

## Hygiene and formatting

> [!WARNING]
> The hygiene grant, the formatter declarations, the `format` command,
> and the related report fields are experimental.

Bearout natively enforces the byte-level hygiene every text file shares
and delegates syntax-aware formatting to programs the repository pins. An
explicit `[hygiene]` grant selects the files: `scope = "repository"` is
every file of the project as Git knows it (the captured index or revision
tree; for the working directory the tracked plus untracked, non-ignored
files), `scope = "declared"` is only the listed `roots` and `files`;
`exclude`, `binary`, and `text` refine any selection by path, and no
extension carries kernel meaning. A file is binary when declared so or
when a NUL byte occurs in its first 8 KiB; binary files are never checked
or rewritten.

```toml
[hygiene]
scope = "repository"
exclude = ["generated"]
binary = ["assets"]

[[formatters]]
name = "python"
command = ["ruff", "format", "--stdin-filename", "{path}", "-"]
extensions = ["py"]
support = ["ruff.toml"]
```

Text rules come from the `.editorconfig` files of the selected tree,
never from the live checkout during an index or revision check: `charset`
(`utf-8` or `utf-8-bom`), `end_of_line`, `insert_final_newline` (exactly
one final newline; an empty file is exempt and never changed), and
`trim_trailing_whitespace` (set it to `false` for Markdown hard breaks). A
value Bearout cannot enforce is reported, not guessed; the supported
subset is documented in [`docs/design.md`](docs/design.md), and complete
EditorConfig compatibility is not claimed.

A formatter is a strict byte transform: the selected file's exact bytes go
to the program on standard input, its standard output is the canonical
form, and a difference is one diagnostic. The program runs from an
argument vector with `{path}` replaced by the project-relative path, from
a private working directory holding only the declared `support` files
read from the selected tree, without color, sequentially, with bounded
streams and a timeout. Running formatters needs `--allow-formatters`,
because a formatter is a trusted host program outside Starlark's
capability model: Bearout confines what it sees, not what it can do, and
checking formatter declarations from untrusted authors is not a supported
security boundary. The formatter's version is an input to
reproducibility; run Bearout inside the environment that pins it, such as
`mise exec -- bearout --allow-formatters check`. Bearout never reads
`mise.toml` and installs nothing. General linting is deferred: linters
produce tool-specific findings that need a separate design.

`bearout format` rewrites selected files of the working directory after
computing every change, applying native normalization before the
formatter, replacing each file atomically with its permissions preserved
and only if it still holds the bytes that were read, and undoing
completed replacements if a later one fails. Nothing is created or
deleted, links are never followed, index and revision sources and
comparison baselines are never formatted, and `generate` never rewrites
sources.

## Contract fixtures

> [!WARNING]
> The fixture vocabulary, the `test` command, and the test report are
> experimental.

`bearout test` proves a repository's policy against controlled mutations
of the selected source without changing anything. An explicit
`[fixtures] files` grant names the fixture files one by one; nothing is
scanned for, and `check`, `generate`, and `format` never execute them.
Each fixture file holds named cases:

```toml
[[cases]]
name = "deleting a rejected record is caught against the unmodified log"
expect = "diagnostics"          # or "clean" or "fatal"
baseline = true                 # compare with the unmodified source
match = "exact"                 # the default; or "contains"

[[cases.mutations]]
delete = "records/decision-0005.md"

[[cases.diagnostics]]
code = "B015"
side = "baseline"
path = "records/decision-0005.md"
rule = "protected-record-deleted"
```

A case derives its candidate from the selected source by applying its
mutations in order through a read-only overlay: `write` replaces or
creates one regular file from inline `content` or a project-relative
`payload` file of the selected source, `delete` removes one regular file,
and `move` relocates one to a path that does not exist. Each path is
touched once per case and never above or beneath another touched path,
nothing beneath a file or through a symbolic link is touched, and every
conflict in any case is refused before the first case runs. Every
case starts from the same unchanged source; the working directory, the
index, Git objects, and the fixture files are never written.

`expect` names the outcome class: `clean` (no diagnostic at all),
`diagnostics`, or `fatal`, optionally with `fatal = "text"` the fatal
message must contain. Expected diagnostics are structured, never rendered
text: `code` is required, and `severity`, `path`, `line`, `side`
(`candidate` or `baseline`), the repository `rule`, and the exact
`message` are optional. They are matched as a multiset, so a repeated
diagnostic needs a repeated expectation. `match = "exact"`, the default,
also fails the case on any diagnostic it did not expect; `match =
"contains"` allows unrelated diagnostics. A contract diagnostic is test
data and fails a case only when unexpected.

With `baseline = true` the unmodified selected source is the comparison
baseline and the overlaid candidate is the candidate, with the Phase 3
authority: the candidate's bootstrap and policy interpret both sides, and
`project["comparison"]` holds the historical view and the change facts.
Policy sees an ordinary project and an ordinary comparison; nothing
exposes fixtures, mutations, or the overlay to Starlark.

The suite, payloads included, is read from the selected source before any
mutation is applied, so `--index` and `--revision <REV>` test exactly what
is staged or committed: an unstaged correction cannot hide a broken
staged fixture, and an untracked payload cannot satisfy an index fixture.
`limits.fixture_cases`, `limits.fixture_mutations`, and
`limits.fixture_bytes` bound a suite. A bootstrap that declares
formatters needs `--allow-formatters` before any case runs; nothing is
authorized silently. There is no `--baseline`: each case decides.

The text report prints one line per case and the details of each failed
case (the outcome mismatch, the fatal message, missing expectations,
unexpected diagnostics); `--format json` prints the test report, a
surface distinct from the contract report, with the source identity,
counts, and every case in suite order. Exit 0 when every case passed, 1
when a well-formed case did not match, 2 when the suite could not run: a
malformed fixture, an invalid mutation, a missing or linked payload, a
repeated case name, an exceeded limit, or a source that cannot be opened.
A broken suite is never reported as a passing one, and a project without
`[fixtures]` is a fatal outcome rather than an empty pass.

Mutation-style tests written in a scripting language, which copy a
repository, edit a file, run the checker, and grep its output, map onto
fixtures case by case: the copy becomes the overlay, the edit becomes a
`write`, `delete`, or `move` (a payload file holds a whole replacement),
the grep becomes a structured expectation, and a test that asserted a
crash becomes `expect = "fatal"`. Tests that mutate directories, run
shell commands, generate random edits, or inspect the checker's text are
outside the vocabulary and stay where they are. No compatibility with any
existing test suite is claimed; the
[`decision-records`](samples/decision-records/) sample shows the shape.

## History and commit policy

> [!WARNING]
> History checks, the `history` command, the history view, and the
> history report are experimental and require the `git` executable.

`bearout history` lets a repository enforce its own commit rules over
exact Git facts. The kernel captures commits, identities, messages,
parents, and changed paths; the repository writes the rules in Starlark.
Conventional Commits headers, allowed types and scopes, header length,
body separation, breaking-change footers, sign-off trailers that must
match the author, and merge or autosquash exemptions are all policy the
repository supplies; Bearout holds no Conventional Commits parser and no
DCO semantics, and it does not verify the legal truth of a sign-off.

```python
def commit_policy(history):
    findings = []
    for commit in history["commits"]:
        if commit["merge"]:
            continue                       # this repository's choice
        author = commit["author"]
        sign_off = "Signed-off-by: %s <%s>" % (author["name"], author["email"])
        if sign_off not in commit["message"].split("\n"):
            findings.append(error(
                "missing `%s`" % sign_off,
                commit = commit["key"],
                code = "sign-off",
            ))
    return findings

history_check("commit-policy", commit_policy)
```

`bearout history range [PATH] [--base REV] [--head REV]` checks the
commits reachable from the head (default `HEAD`) but not from the base:
Git's `base..head`, or everything reachable from the head without a
base. Both names are resolved exactly once and recorded with their full
identities; the base itself is excluded; merge commits are included and
policy decides whether they matter. Nothing is read from `BASE`, `HEAD`,
or any provider variable, and an all-zero base is not special: omit
`--base` for a new branch. Commits are exposed oldest first in a
deterministic topological order, the full object identity breaking ties
among simultaneously eligible commits. Each commit's changes are
relative to its first parent, or to the empty tree for a root commit,
without rename detection: a rename is a removal plus an addition whose
object identities policy may compare. Paths are repository-relative
facts, with a project-relative form when the path lies inside the
Bearout project.

`bearout history message [PATH] --file FILE` is the commit-msg hook path.
It reads exactly the named message file, which must be a regular,
non-linked file inside the repository's resolved Git directory (the
linked worktree's own directory in a worktree), bounded before it is
read, valid UTF-8, and free of NUL; comments, scissors lines, autosquash
prefixes, and blank lines reach policy exactly as Git supplied them. The
author is the identity Git would record, the parents are `HEAD` and any
merge in progress, and the staged changes come from the same captured
index that supplies the policy. An empty message is an input to policy.

Authority is explicit: a range reads `bearout.toml`, the entry module,
and every loaded module from the resolved head's tree; a pending commit
reads them from the captured index. An unstaged policy edit cannot
change a commit-msg check, and the working tree cannot override a range
check. Only history checks run: no resource discovery, documents,
hygiene, ordinary checks, generators, formatters, or fixtures. Identities
are the commit object's own, without `.mailmap`, case folding, or any
inference that author and committer are one person; messages are
byte-exact. Signed commits parse with their signature headers kept out
of the message. Missing objects are never fetched, and a range that
reaches a shallow boundary is refused rather than described as complete
history.

A history finding targets a commit key from the view (`pending` for the
pending commit) with an optional message line, or nothing for a
range-wide finding; a commit target never combines with a resource,
path, or comparison side, and ordinary checks cannot target commits.
Accepted findings are B032 (error) and B033 (warning) in a distinct
history report, rendered as `commit <id>:<line>:B032[rule]: ...`,
`commit pending:...`, or `range:...`, with the registered check name as
the rule identity unless the finding carries its own `code`. Script
diagnostics sort first by path, then range-wide findings, then commit
findings in commit order; within a target by line, code, rule, and
message. Exit 0 with no finding, 1 with any finding, warnings included,
2 for invocation, Git, policy-loading, malformed-history, or limit
failures. An invalid revision or incomplete history is never a policy
finding.

`limits.history_commits`, `limits.history_changes`,
`limits.history_commit_bytes`, and `limits.history_bytes` bound a run.
Fixture cases may supply a synthetic pending message with
`[cases.history]` and expect findings by `commit`, so Conventional
Commit and sign-off policies are regression-tested without a Git
repository; the [`commit-policy`](samples/commit-policy/) sample shows a
complete policy with its fixtures.

A commit checker written in a scripting language, which runs `git log`
or `git show`, parses the text, and prints failures, maps onto this
model: the parsing becomes the history view, each rule becomes a branch
of the history check reading `subject`, `message`, `author`, `parents`,
and `changes`, each failure becomes `error(..., commit = key)`, and the
CI job becomes `bearout history range --base <base>` while the hook
becomes `bearout history message --file "$1"`. Rules that depend on
branch names, remote state, signature verification, or provider APIs
have no facts to read and stay where they are. No compatibility with any
existing checker is claimed.

## Samples

The repository's [`samples/`](https://github.com/malekpour/bearout/tree/main/samples)
directory holds nine complete projects, from three linked notes to a
spreadsheet-expression language whose Rust lexer, parser, and conformance
tests are generated from the resource graph and compiled in CI. The
[samples index](https://github.com/malekpour/bearout/blob/main/samples/README.md)
is the capability matrix. Every sample is checked, and its outputs
verified, by the test suite. Samples are not part of the crates.io package.

## Development

All required tools and versions are pinned in
[`mise.toml`](https://github.com/malekpour/bearout/blob/main/mise.toml).

```sh
mise run setup
mise run fmt
mise run check
```

See [`CONTRIBUTING.md`](CONTRIBUTING.md), [`docs/design.md`](docs/design.md),
and [`docs/technology-evaluation.md`](docs/technology-evaluation.md).

## Trust

Bearout is a capability-confined host with resource limits. It is not a
sandbox for hostile repositories; see [`SECURITY.md`](SECURITY.md).

## Licence

Licensed under the [Apache License 2.0](LICENSE).

Copyright 2026 Ali Malekpour.
