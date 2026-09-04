# Design

Bearout is a deterministic repository contract engine. A *contract* here is
a machine-checkable agreement about the resources in a repository: their
envelope, their shape, the relations between them, and the artifacts
generated from them. It is not necessarily a legal contract; two of the
samples model commercial and multilateral records, but the engine knows
nothing about law.

## Kernel responsibilities

The Rust kernel owns everything that must be the same for every adopting
repository:

- project sources: the live working directory through a filesystem
  capability, or a frozen Git index or revision, read through one
  read-only tree interface;
- discovery of resources beneath the declared roots, sorted, without
  following symbolic links;
- discovery of schema-less Markdown documents exactly where the bootstrap
  selects them;
- parsing of the resource envelope: TOML front matter through `toml_edit`
  over an exact byte range, and the Markdown body through Comrak, which
  also parses schema-less documents;
- structural validation against JSON Schema 2020-12 shapes and the
  `x-bearout` vocabulary;
- graph construction: identifier index and typed relations;
- Markdown reference checking: links, images, heading anchors, and explicit
  anchors across resources and documents;
- the Starlark runtime: contained loading, resource limits, cancellation,
  the ABI, and immutable views;
- diagnostics with stable codes and deterministic ordering;
- generation: plan validation, rendering, provenance, the state manifest,
  and confined, staged delivery through a filesystem capability.

## Repository responsibilities

Everything domain-specific belongs to the repository:

- schema identifiers and their shapes;
- validators, project checks, and generators in Starlark;
- templates;
- naming conventions such as "file stem equals resource id", which the
  samples enforce as policy and the kernel does not.

The kernel never learns a repository's semantics. When two real consumers
need the same mechanism, it can move into the kernel; until then it stays
in policy.

## Phase ordering

Every run proceeds through these phases in order:

1. **bootstrap**: open the selected source as a read-only tree, parse
   `bearout.toml` from it, validate the roots;
2. **discovery**: walk the resource roots, then collect the selected
   schema-less documents minus the paths resources claimed;
3. **parsing**: envelope, body structure, fragments; document text and
   structure;
4. **policy load**: the Starlark entry module and everything it loads,
   which registers schemas, checks, and generators;
5. **structural validation**: shape, required sections, fragment shapes;
6. **graph construction**: identifiers from every parsed resource,
   relations from structurally valid ones; then Markdown references from
   structurally valid resources and parsed documents, resolved against the
   tree and the discovered Markdown set;
7. **repository policy**: validators over structurally valid resources,
   then checks over the whole graph only when no error has been reported;
8. **generation planning**: only when no error has been reported;
9. **rendering**: every artifact into memory, with digests;
10. **delivery**: comparison against the state manifest, then atomic
    per-file replacement.

A resource that fails parsing or structural validation is never handed to a
validator. Its identifiers still resolve, so a reference to it does not
produce a second, cascaded diagnostic. Checks and generators receive only a
structurally valid graph.

## Capability boundary

`bearout.toml` is static and is the security boundary of a project. It
names the entry module and grants four kinds of root: resource roots, the
rules root beneath which `load()` resolves and shapes live, the templates
root, and the output roots. Roots are disjoint, none is the project root,
and the bootstrap itself lies beneath none of them. Repository policy can
register schemas, checks, and generators but cannot widen any grant.

All working-directory access goes through a `cap-std` directory capability
opened on the project root; the kernel holds no ambient path. Output
delivery refuses absolute paths, parent traversal, paths outside the output
roots, symbolic links anywhere in the output path, and files that Bearout
does not own according to the state manifest.

## Project sources

Every phase before delivery reads the project through one internal
read-only interface, the read tree: bytes and UTF-8 text of a file, file
length, file, directory, and generic existence (following links), the
first symbolic link on a path (not following), deterministic recursive
walking that never follows links or enters submodules and fails on a name
that is not a portable project path, and a subtree view rooted at a
directory. The interface carries no write or delete operation. Writes go
through a separate delivery capability that only the working directory
provides, so checking and generation planning depend on reads alone,
`generate --check` runs against any source, and writing generation
requires the working directory by construction.

The source is selected before anything is read. The Git-backed sources are
experimental and require the `git` executable.

**Working directory.** The live filesystem through the `cap-std`
capability, with its existing concurrency semantics: Bearout makes no
snapshot, and a concurrent edit is visible to a run. It is the only source
that can hand out the delivery capability.

**Git index.** The index of the repository that owns the project root,
captured once when the run starts as a frozen set of paths, modes, and
object identities: the tree a commit would record. The index file is
copied into a private temporary file first, and every listing of the
capture (`ls-files --stage` and both `diff-index --cached` views) reads
that copy, so the entries and the intent-to-add classification describe
one authoritative state even if the live index changes during the run;
the copy is removed afterwards. Staged additions and modifications are
present; unstaged modifications, untracked files, and staged deletions are
absent; a staged rename appears only at its destination; modes are those
of the index. An unmerged entry fails the capture rather than silently
choosing a stage. An intent-to-add entry, which `git commit` would not
record, is excluded; it is identified as an entry that Git's
`--ita-invisible-in-index` view treats as absent, which holds even after
the working-tree file is removed. `GIT_INDEX_FILE` is honoured only when
it names a regular file, not a symbolic link, whose canonical path lies
directly inside the repository's applicable Git directory (the worktree's
own for a linked worktree), so a partial-commit hook sees the index being
committed and a stale, foreign, or redirected value is ignored. The index
is never written and nothing is checked out.

**Revision.** Any commit-ish or tree-ish Git resolves. The name is resolved
exactly once; the resolved tree identity is retained, reported, and used
for the rest of the run even if a branch or tag moves. A name that does not
resolve, or that names a blob, is a fatal outcome. A revision expression is
passed to Git after `--end-of-options` and may not begin with `-`.

Both Git sources share one tree model. Repository and project roots are
distinct: the owning repository is discovered from the project root, the
project's prefix within it is determined once, and only paths beneath that
prefix are exposed, so a project below the repository root and a linked
worktree (where `.git` is a file) work alike. Each captured entry retains
its kind (regular file, executable file, symbolic link, directory explicit
or inferred, submodule gitlink), its object identity as an opaque
hexadecimal string of whatever length the repository's hash algorithm
produces, and its size where Git reported one. Blob content is loaded on
demand by object identity through a long-lived `cat-file --batch` process,
cached for the run only, and read exactly as stored: no working-tree
filters, line-ending conversion, or smudge transformations. Listings and
blobs are bounded in size, and a read error on either is a fatal outcome,
never an empty result. Git is run as a fixed executable with an argument
vector and a fixed environment: every variable that redirects the
repository, its objects, or its configuration (`GIT_DIR`,
`GIT_WORK_TREE`, `GIT_COMMON_DIR`, `GIT_INDEX_FILE` except as above,
`GIT_OBJECT_DIRECTORY`, `GIT_ALTERNATE_OBJECT_DIRECTORIES`,
`GIT_NAMESPACE`, `GIT_REPLACE_REF_BASE`, the `GIT_CONFIG*` family), that
changes discovery (`GIT_CEILING_DIRECTORIES`,
`GIT_DISCOVERY_ACROSS_FILESYSTEM`), that alters pathspecs, or that traces
to arbitrary files is dropped, so discovery always starts from the project
root; replacement objects (`GIT_NO_REPLACE_OBJECTS`) and lazy fetching
from a promisor remote (`GIT_NO_LAZY_FETCH`) are disabled, opportunistic
index writes and prompts are off, and messages use the C locale. Git's
error output is reduced to one bounded, control-character-free line before
it reaches a report.

Every capture carries a deterministic digest: BLAKE3 over one line per
file, link, and gitlink beneath the project (`<mode> <object> <path>`, in
path order; directories excluded, since the index infers them and a tree
lists them), plus one line per directory holding a non-portable name.
Identical content digests equally whether it came from the index or from
a revision, which is what a later candidate/baseline comparison needs. It
is not a Git object identity.

A symbolic link inside a Git tree resolves lexically against the link's
directory and only inside the tree it is read from: an absolute target, a
target that leaves the project or the templates subtree, a chain of more
than forty hops, a missing target, and traversal through a gitlink are all
refused, and the working filesystem is never consulted. Discovery skips
link entries, rule modules and shapes refuse to be reached through links in
every source, and a submodule is never entered: it exists as an entry, is
not a directory, and nothing beneath it is readable.

The JSON report carries a `source` field for the Git sources only
(`{"kind": "index", "digest": ...}` or `{"kind": "revision", "revision":
..., "tree": ..., "digest": ...}`), so that a report can be tied to the
exact content it examined. The field is experimental. Repository policy is
unaware of the source: views are identical across sources. The tree
interface holds two independent trees in one run when a comparison is
requested; see the comparison section below.

## Candidate and baseline comparison

Comparison is opt-in and experimental. `Options::baseline` (`--baseline
<rev>`) names one exact Git revision of the same repository; the kernel
never infers `HEAD`, a parent, a merge base, or a default branch. The
candidate is the selected source as usual, working directory, index, or
revision, and is checked exactly as without a comparison. The baseline is
opened before the bootstrap is read, resolved once, never written, and
dropped with the run. Writing generation still requires a working-directory
candidate; `generate --check` compares from any candidate.

**Authority.** The candidate's policy is the only policy executed. The
baseline's `bearout.toml`, when present, is parsed as passive historical
data whose only effect is to say which paths that revision classified as
resources and as schema-less documents, with resource precedence applied
on each side independently; it grants nothing, and no baseline rule
module, shape, template, generator, or output state is loaded or executed.
The candidate's limits bound both sides and can only be tightened by the
candidate. The candidate's registered schemas and shapes validate both
sides, so a baseline resource whose schema the candidate no longer
registers, or that fails the current shape, is reported and withheld from
policy rather than exposed unvalidated: the current policy must retain
enough schema knowledge to interpret the history it compares against.
Validators run once per candidate resource, never per baseline resource;
comparison is a project-level concern. The baseline's identifier graph is
rebuilt, so duplicate historical identifiers and unresolved or mistyped
typed relations are reported on the baseline side, because policy pairs
records through that graph; the baseline's Markdown links, images, and
anchors are not re-checked against either tree, and no generation runs
against the baseline. A revision that predates the project directory or its
`bearout.toml` is an empty historical project, so a wholly added project
compares; a malformed historical `bearout.toml`, or one naming roots and
files its tree lacks, is fatal, since it leaves the historical
classification unknown.

**Diagnostics.** Baseline problems keep the codes of the same failure
classes and carry a structured side: `"side": "baseline"` in JSON, absent
for the candidate, and a `baseline:` prefix in text. Report order places
every candidate diagnostic before every baseline diagnostic. A baseline
error fails the run and, like any other error, stops project checks from
running: a comparison against history the policy cannot interpret is not
made.

**Change facts.** Each side records, for every file it actually read (the
bootstrap, the discovered resources, the discovered documents), the
classification its own bootstrap gave the path and the BLAKE3 digest of
exactly the bytes parsed, so a digest and its parse come from one read
even for the live working directory. The two surfaces are compared by
path: `added`, `removed`, or `modified`, a differing classification
counting as a modification, unchanged paths omitted, no Git rename or
similarity heuristics, so a rename is a removal plus an addition while
resources still pair through their stable ids in the two views. Documents
stay path-identified. This is the declared contract surface, not a
repository diff; file modes, commit metadata, and commit ranges are not
part of it.

**Views and findings.** `project["comparison"]` is `None` without a
baseline; otherwise `baseline` holds the revision as supplied, the
resolved tree, the tree digest, and `resources`, `by_id`, `by_schema`,
`ids`, and `documents` with the candidate's value shapes, sorted the same
way, and `changes` holds the facts. Only structurally valid baseline
resources and parsed baseline documents appear. A check may target either
side with `side`, so deletion or corruption of a history-only resource or
document can be named; a resource on both sides is addressed through the
side, never guessed. Validators stay confined to their own candidate
resource. Nothing exposes the filesystem, arbitrary historical blobs,
Git, process state, or the source a run reads. The kernel enforces no
immutability: which records are protected, from when, which fields or
fragments, which corrections are allowed, and whether deletion, movement,
or reclassification is permitted are all repository policy, as the
`decision-records` sample shows.

## Schema-less documents

A resource has an envelope, a schema, an identifier, a shape, relations,
and an optional Markdown body. A schema-less document has a project path
and Markdown structure, nothing more: no schema or identifier is
synthesized, and a malformed resource never silently becomes a document.
The bootstrap selects documents explicitly, as `[documents] roots`
(walked recursively for `.md` files, never following links or entering
submodules, failing on non-portable names like resource discovery) and
`[documents] files` (named one by one, which must exist, be `.md`, and not
be reached through a link). Both lists are sorted; duplicates, nested
roots, and an empty table are errors. The grant is read-only and may
overlap resource, rules, templates, or output roots without changing what
generation may write. A path selected as both resource and document is
processed once, as a resource. Documents are bounded by `limits.documents`
(default 10,000, fatal when exceeded) and `limits.document_bytes` (default
4 MiB, B022 per document), separately from resources. A document that
cannot be read or is not UTF-8 is B022; a leading byte-order mark is
removed; CRLF line endings keep their line numbers.

Documents and resource bodies share one Comrak model: headings with GFM
anchors (Comrak's own algorithm, duplicate-heading suffixes included),
explicit anchors from the `id` and `name` attributes of `<a>` elements in
raw HTML (attribute order and case do not matter; no other HTML is
interpreted, and HTML links and images are not collected), fenced blocks,
links with visible text, and images with alt text, from inline and
reference-style syntax and never from code.

Reference checking is a document concern and lives outside the identifier
graph. A target with a URL scheme is not local. A relative target resolves
from the source's directory, a leading `/` from the project root, `.` and
`..` never leaving the project; the query string is dropped; percent
escapes are decoded on bytes and the result is revalidated as a project
path. A bare `#fragment` resolves within the source; a fragment on another
Markdown file resolves against its heading and explicit anchors when that
file is a structurally valid resource or a parsed document, produces
nothing when the file failed an earlier phase (that failure is already
reported), and is reported when the file exists but was never selected, so
that no anchor is claimed valid without having been read. Fragments on
non-Markdown files and on directories are not interpreted. An existing
file or directory is a valid link target; an image must name an existing
file. Symbolic links and submodules keep the tree's rules. Every broken
reference is one B011, with distinct wording for links and images.

Repository policy sees documents as `project["documents"]`, in path order,
each with its path, text, line count, sections, anchors, links, and
images, and may report a finding against a document `path` and a line
within it; a validator remains confined to its own resource. Which
documents matter, and what a good link or alt text is, are repository
decisions: the kernel assigns no meaning to a README, a governance file,
or any other name.

## Repository hygiene and formatting

Everything here is experimental. The boundary is hybrid: the kernel
enforces the byte and text hygiene every file shares, because it needs no
knowledge of a language and must be identical across sources; syntax-aware
formatting, indentation, wrapping, quoting, and import order belong to
external programs the repository selects and pins, because embedding one
language's rules would make the kernel a formatter for that language. No
extension has kernel meaning; no linter runner exists, since linters emit
tool-specific findings that need their own design; and Bearout never
parses `mise.toml` or installs anything. Only the candidate is selected;
the comparison baseline is neither checked nor formatted, and the
comparison surface stays what Phase 3 defined.

**Selection.** `[hygiene] scope = "repository"` is every file of the
project as Git knows it: for a captured index or revision, that tree's
regular files; for the working directory, the tracked plus untracked,
non-ignored paths that Git lists through the hardened runner, kept only
while they exist as regular files, so a tracked file deleted from disk is
absent. Staged deletions are absent from the index, unstaged edits cannot
reach it, and untracked files cannot satisfy it. A repository-wide
selection outside a Git repository is fatal; `scope = "declared"` walks
listed roots and names listed files without Git. `exclude`, `binary`, and
`text` refine by path prefix, every list is sorted, links are never
followed, submodules never entered, the project prefix confines
discovery, and `limits.files` bounds the count. Each selected file is read
once, bounded by `limits.file_bytes` and by what remains of
`limits.hygiene_bytes`, the total for every hygiene input of the run
(selected files, `.editorconfig` files, and formatter support files
together), whichever is smaller. Every byte a read pulls is charged, the
one-byte overflow probe of a rejected read included; a frozen Git tree
rejects an over-limit blob from its recorded size without loading it. The
boundary reached is the one reported: a file too large or unreadable is
B024, and an exhausted total is fatal.

**Text hygiene.** Properties come from `.editorconfig` files of the
selected tree only, parsed by `ec4rs` from the bytes that tree holds:
every file between the project root and the selected file applies, the
innermost `root = true` ends the search, closer files win, and the project
root is the outer boundary. The enforced subset is `charset` (`utf-8`,
`utf-8-bom`), `end_of_line` (`lf`, `crlf`, `cr`), `insert_final_newline`,
and `trim_trailing_whitespace`; every other property is ignored, and a
supported property with a value Bearout cannot enforce (`latin1`,
`utf-16le`, a misspelled value) is B023 on the file rather than a guess.
An unusable `.editorconfig` is B023 once, on that file, and suspends
checks beneath it. Bearout's own decisions: a file is binary by
declaration or when its first 8 KiB contain a NUL, an empty file is text,
binary files are never checked; a text file must be valid UTF-8 even with
`charset` unset, because undecodable bytes cannot be checked line by
line; `insert_final_newline = true` means exactly one final newline, so
trailing blank lines are violations, and an empty file satisfies either
setting and never changes. Each aspect is one diagnostic per file naming
the first line: B025 encoding, B026 line ending, B027 final newline, B028
trailing whitespace; an encoding failure stops the file's check so nothing
cascades. Identical bytes give identical diagnostics from every source.

**External formatters.** A `[[formatters]]` entry is an executable plus an
argument vector, never a shell, with `{path}` replaced by the
project-relative path; `paths` and `extensions` assign it selected files,
and a file may have at most one formatter. The protocol is a byte
transform: the file's exact bytes from the chosen tree on standard input,
canonical bytes on standard output, B029 when they differ. The program
runs from a private temporary directory containing only the declared
`support` files read from the selected tree, so a staged or committed
configuration governs an index or revision check even when the checkout
differs; every temporary and cache location it is told about lies outside
the target repository; it runs non-interactively with color disabled,
sequentially in path order, with bounded standard input, output, and
error, and a wall-clock bound after which it is killed and reaped. A
non-zero exit, timeout, oversized output, or abnormal end is B030 on the
file; a program that cannot start is fatal. Formatters run only when the
host authorizes them (`--allow-formatters`, `Options::allow_formatters`);
declaring them without authorization is fatal rather than silently
skipped, and nothing about them reaches Starlark.

**Trust boundary.** An authorized formatter is a trusted host program. It
is not confined by Starlark's capability model, Bearout is not a security
sandbox, and checking external-tool declarations from untrusted authors
is not a supported security boundary: Bearout controls what the program
receives and where it starts, not what it can read or write elsewhere.
The program's version is an input to reproducibility that Bearout does
not detect; the repository runs Bearout inside its pinned environment.

**Formatting writes.** `bearout format` is the only operation that
rewrites user-owned files; `check` and `generate --check` never write, and
`generate` never rewrites sources. The write requires the working
directory and refuses a comparison baseline. Every transformation is
computed first: native normalization in a fixed order (byte-order mark,
line endings, trailing whitespace, end of file), then the assigned
formatter over the normalized bytes. Only existing, selected regular files
change; nothing is created or deleted; a link is never followed or
replaced; permissions, the executable bit included, are preserved; a
file is replaced only if it still holds the bytes that were read; each
replacement is atomic through the working-directory writer; a failure
part-way undoes completed replacements from a journal, reporting
restoration failures (B031); and no temporary file remains. The
generated-output manifest plays no part: the command itself is the
authorization to change these files.

## Contract fixtures

Everything here is experimental. `bearout test` is a general facility for
proving a repository's policy against controlled candidate mutations; the
vocabulary encodes no repository's records, statuses, or naming rules,
and the kernel learns nothing about what a case means.

**Declaration.** `[fixtures] files` names fixture files one by one:
sorted, unique, portable, `.toml`, never a Bearout manifest, and never
beneath a resource or output root where discovery or delivery would treat
them as something else. Nothing is scanned for, and a project without the
grant has nothing to test: `bearout test` is fatal there rather than an
empty pass. Fixture files and payloads are read from the selected source
through the same read tree as everything else, within
`limits.fixture_bytes`, never through a symbolic link; `check`,
`generate`, and `format` never read them.

**Case model.** Each file holds `[[cases]]` in file order; suite order is
fixture files sorted, then file order, and case names are unique across
the suite. A case has an ordered list of mutations, whether the unmodified
source is supplied as the comparison baseline, an expected outcome class
(`clean`, `diagnostics`, `fatal`), structured expected diagnostics, and an
explicit matching mode. The mutation vocabulary is the smallest useful
one: write or replace one regular file (inline UTF-8 `content` or a
project-relative `payload` file of the selected source), delete one
regular file, move one regular file. Directory moves, recursive deletion,
modes, links, gitlinks, scripts, templating, environment expansion, and
random mutation generation are out of scope.

**Overlay.** A case's candidate is a read-only overlay over the selected
tree holding only the written bytes, tombstones for deletions and move
sources, and move destinations that read the source's bytes from the
unchanged base. No repository copy is materialized, and every read goes
through the base, so the overlay has no ambient filesystem access and no
write authority reaches the checking pipeline. It implements the same
observable tree semantics as the other sources: sorted walks that never
follow links or enter submodules, file and directory existence (a
directory exists when the base has it or a written file lies beneath it),
subtree confinement through the base's own subtrees, and bounded reads
that report the bytes pulled. Every case's mutations are validated, and
its overlay built, before any case is evaluated, so an invalid later case
stops the suite before an earlier case runs the policy or an authorized
formatter. Validation proceeds in manifest order over the virtual state
the earlier mutations produced: each path is touched once per case and
never lies beneath or above another touched path (directory replacement
is not implemented, so no path can end up both a file and a directory),
a write replaces a regular file or creates one where nothing exists, a
delete and a move source must name a regular file of the base, a move
destination must not exist, and no touched path may be, lie beneath, or
be reached through a link, a submodule, or a regular file. Every case
starts from the same unchanged base, so nothing
leaks between cases; the repository-wide hygiene universe of a
working-directory source is Git's listing plus the overlay's written and
moved files, filtered by what the overlay presents. The whole suite,
payloads included, is captured before any mutation is applied, so a
mutation cannot alter which cases run or what they write.

**Authority.** A case with `baseline = true` compares the overlaid
candidate with the unmodified selected tree under the comparison
semantics above: the candidate's bootstrap, limits, policy, and shapes
interpret both sides, the baseline is passive historical data, and the
comparison view identifies the baseline with whatever identity the
source has (tree and digest for a revision, digest for the index, none
for the working directory). Each case is one evaluation of the same
single-run engine `check` uses, over the overlay and the optional
baseline, with `Command::Check`: no generation planning, rendering,
delivery, or formatting write ever runs from a fixture. Policy observes an
ordinary project and an ordinary comparison; nothing exposes fixture
state, mutations, or the overlay to Starlark, and the fixture runner adds
no filesystem, process, environment, network, clock, or random access.

**Expectations.** The outcome class of a candidate is `fatal` when its
evaluation failed, `diagnostics` when it reported anything (warnings
included), `clean` otherwise. A `fatal` expectation may pin the message
with `fatal = "text"`. Expected diagnostics name fields, never rendered
text: `code`, and optionally `severity` (which must agree with the code),
`path`, `line`, `side`, the repository `rule` identifier, and the exact
`message` as a deliberately brittle assertion. Matching is a multiset
assignment: each expectation consumes at most one diagnostic and each
diagnostic satisfies at most one expectation, found as a deterministic
maximum bipartite matching from expectations in declaration order and
diagnostics in report order, so the result never depends on how
expectations overlap. `match = "exact"`, the default, fails the case on
any unmatched diagnostic; `match = "contains"` allows unrelated ones. A
contract diagnostic is test data; only an unexpected one, a missing one,
or a mismatched outcome class fails a case.

**Three failure kinds.** A contract diagnostic that a case expects is a
pass. A well-formed case whose result does not match is an assertion
failure: the suite completes, the case is reported with its expected and
actual outcome classes, missing expectations, unexpected diagnostics, and
actual fatal message, and the exit code is 1. A suite that cannot run is
fatal with exit code 2 and reports no case at all: malformed fixture
syntax, an invalid mutation, a missing or linked payload, a repeated case
name, a source or tree that cannot be opened, an exceeded fixture limit,
or a formatter declaration without `--allow-formatters`. Assertion
failures carry no B-series code: those describe checked repository
findings, and fixture mismatches are a separate machine-facing surface,
the test report, which is deterministic in text and JSON and valid JSON
on every exit path.

**Sources and limits.** The suite reads from the working directory, the
index, or a revision with the hardened semantics above; the suite, the
policy, resources, documents, and payloads all come from that one tree.
There is no `--baseline`; each case decides. `limits.fixture_cases`
bounds the cases of a suite, `limits.fixture_mutations` the mutations
across it, and `limits.fixture_bytes` the bytes of fixture files and
payloads read, so a repository cannot create effectively unlimited engine
evaluations through fixtures. Formatters keep their authorization
boundary: a bootstrap that declares them is refused before any case runs
unless `--allow-formatters` is given, and a case whose mutation declares
them is an observable fatal outcome of that candidate, never a silent
authorization.

## Repository history and commit policy

Everything here is experimental. The kernel establishes exact Git facts
and runs repository-owned history checks over them; it contains no
Conventional Commits parser, no allowed types, no DCO semantics, no merge
exemptions, and no branch conventions.

**Commands and modes.** `bearout history range` resolves the head
(default `HEAD`) and an optional base exactly once to commits, through
the hardened Git runner, and inspects the commits reachable from the head
but not from the base, or everything reachable from the head without a
base. The base itself is excluded, merges are included, an all-zero base
is not special, and no environment variable names a revision. A
hyphen-led, malformed, non-commit, missing, or ambiguous name is fatal.
`bearout history message --file` describes the commit a `commit-msg`
hook is about to make from one captured index.

**Authority.** A range reads the bootstrap, the entry module, and every
loaded module from the resolved head's tree; a pending commit reads them
from the captured index, honouring a valid alternate `GIT_INDEX_FILE`
under the source rules above. The working tree never supplies policy to
either, and the facts come from the same repository as the policy.
Projects below the repository root and linked worktrees work. The
history command loads policy but runs none of the contract pipeline: no
discovery, validation, documents, hygiene, ordinary checks, generators,
formatters, or fixtures, so a hook check is never coupled to unrelated
working-tree state. A missing or malformed bootstrap, a missing entry
module, a policy that does not load, and a policy that registers no
history check are all fatal.

**The history reader.** Git runs with argument vectors and the fixed
environment above: no shell, no prompts, no replacement objects, no lazy
fetching, bounded output, sanitized errors, and children killed and
reaped on every failure. Raw commit objects are read through one
long-lived `cat-file --batch` process, each rejected from its announced
size before a byte is loaded when it exceeds `limits.history_commit_bytes`
or the remaining budget; continuation headers such as `gpgsig` and
`mergetag` belong to the header above them and never reach the message.
A commit with a non-UTF-8 header or message, a declared non-UTF-8
encoding, a malformed identity, or a non-portable path is fatal and
names the commit. Missing objects in a partial clone fail rather than
fetch. A range whose set contains a shallow boundary commit is refused,
because the history reachable from it is cut off; an explicit range
above the boundary runs, since every commit and tree it needs is local.
Rename detection is off regardless of configuration, and `.mailmap` is
never applied.

**Set and order.** The set is Git reachability; the order is Bearout's:
oldest first, a commit becoming eligible once every parent inside the
set is emitted, the smallest full object identity among the eligible
going first. Changes within a commit sort by repository path.

**Facts.** Each commit carries its key (the full identity, or `pending`),
identity, tree, ordered parents, the derived `merge` flag, the raw author
and committer identities (name, email, Unix timestamp, numeric offset,
exactly as the object records them), the byte-exact message and its
first line, and its changes relative to the first parent, or the empty
tree for a root commit: repository-relative path, project-relative path
when inside the project, `added`, `removed`, `modified`, or
`type-changed`, and the mode, object identity, and kind (`file`,
`executable`, `symlink`, `gitlink`) of each side. A rename is a removal
plus an addition; identical object identities are observable facts, not
a claim of intent. Merge, fixup, squash, amend, revert, and root commits
are all present. The pending commit has key `pending`, no identity,
tree, or committer, `HEAD` and any `MERGE_HEAD` as parents, the exact
message file, the author Git would record, and the staged changes of the
captured index against `HEAD` or the empty tree on an unborn branch.

**Message file.** Exactly the named file is read: a regular file, not a
link, whose canonical location lies inside the repository's resolved Git
directory (a linked worktree's own directory in a worktree), no larger
than `limits.history_commit_bytes` before it is opened, valid UTF-8, and
free of NUL. Nothing is stripped: comments, scissors sections, autosquash
prefixes, and blank lines are policy's to read. An empty message is an
input to policy.

**Policy and findings.** `history_check(name, function)` registers a
check that runs only for the history command and history fixture cases,
under the contained loader and every Starlark limit, with no filesystem,
Git, environment, process, network, clock, or random access; the view is
immutable and there is no regex facility. `error()` and `warning()` take
`commit=`: a key present in the view (`pending` only for a pending
check) with a `line` within that commit's message, or nothing for a
range-wide finding, which then carries no line. A commit target is
exclusive with a resource, a path, and the baseline side; ordinary checks
and validators cannot target commits; history checks cannot name a
resource or document. Accepted findings are B032 and B033 with the
registered check name as the rule identity unless the finding carries
its own code; loading, execution, output, and malformed-result problems
keep B012, B013, B014, B017, and B018 against the script path. A commit
identity is never encoded as a path.

**Report and order.** The history report carries `ok`, the mode, the
policy source (the resolved head as a revision, or the captured index),
the supplied and resolved base and head, the commit count, the findings,
and any fatal outcome; JSON is valid on every exit path. Findings sort by
target, script paths first (by path), then range-wide, then commits in
commit order; within a target by line, code, rule, and message; then
deduplicated. Any finding, warnings included, exits 1; a fatal inability
to establish the facts or load the policy exits 2 and is never a policy
finding.

**Limits.** `history_commits` (10,000) bounds the commits of a run, which
covers a large release range while a pull request inspects tens;
`history_changes` (100,000) bounds the changed paths across the run;
`history_commit_bytes` (64 KiB) bounds one commit object, headers and
message together, and the pending message file, well above signed
commits with long bodies; `history_bytes` (64 MiB) bounds everything read
for the facts, listings and commit objects included, every listing read
within what remains. The view is bounded transitively by these; the
facts are captured in full before policy runs.

**Fixtures.** A fixture case may replace mutations with `[cases.history]`,
a synthetic pending commit built by the same constructor and admitted by
the same rules as the real command, checked by the registered history
checks alone, with no Git call and no external program. It is exclusive
with mutations and a comparison baseline, bounded by the fixture and
history limits, and matched by the unchanged exact and contains
semantics with `commit` as a structured expectation field. Range
topology and changed-path policy stay covered by Bearout's own
synthetic-repository tests; a declarative history DAG fixture is deferred.

## Schema and resource identity

A schema identifier is `<namespace segments>/<kind>@<major>`, lowercase
kebab-case, at least one namespace segment, positive major. Identifiers
are repository-owned and never centrally registered; the kernel only
requires them to be well formed and registered by the project's policy.

Resource and fragment identifiers share one namespace per project and are
lowercase kebab-case. A fragment's kind is `<schema>#<fragment kind>`, which
is also how a relation names fragment targets.

The envelope keys `schema`, `id`, and `refs` are owned by the kernel and
validated by it. Every other front-matter key is a repository field that a
shape validates. A shape may not declare the envelope keys.

## Starlark ABI

See [starlark-abi.md](starlark-abi.md). The ABI is version 0 and
experimental. Scripts receive frozen dict views and return lists of host
values constructed by `error()`, `warning()`, and `output()`; every field is
checked at construction and again on admission. Nothing is decoded
permissively, and an invalid target is a diagnostic against the script, not
a silently reattributed finding.

## Diagnostic stability

See [diagnostics.md](diagnostics.md). Codes, severities, and report
ordering are the machine-facing surface. Until the first tagged release
they are experimental; after it, a code is never reused for a different
meaning and removals are announced in the changelog.

## Generated-output lifecycle

Generation is staged. Every plan entry is validated, every artifact is
rendered into memory, and digests are computed before anything touches the
tree. `bearout-state.toml` at the project root records, for every output
Bearout owns, the generator, template, content digest, and an input digest
over the Bearout version, the template source, and the context. The
manifest is parsed strictly: it is absent, valid, or invalid, and an invalid
manifest stops the run before any file is touched. A manifest with no
`outputs` entries omits the key, which is the serializer's own form for an
empty manifest and is accepted; every other omission is an error.

Ownership is proven only by the manifest. An existing file at a planned
path that the manifest does not own is never overwritten, even when its
bytes already equal the rendered bytes; generation must begin with the
path absent for ownership to be established.

Delivery is one journaled transaction: changed outputs are written, owned
and unmodified orphans are removed, then the manifest is written. Each file
replacement is atomic: an exclusively created, uniquely named temporary
file in the same directory is written through its open handle, its data is
synced, and it is renamed into place, so a symbolic link is never followed
or installed and a reader sees either the old or the new file. That is the
whole guarantee. The multi-file sequence is not atomic: if a step fails,
every completed step is undone from the journal where possible, both the
failure and any restoration failure are reported, and the manifest is never
written, so the manifest never claims a delivery that did not complete.
This is in-process rollback, not crash consistency. Bearout does not sync
directories, so a power failure or kill between two renames can leave some
outputs new and some old with the previous manifest still in place; the
next `bearout generate --check` reports exactly that as stale outputs, and
a normal run repairs it because every such file is still owned.

Reads of rules and shapes refuse paths that pass through a symbolic link,
in every source. Templates are read through a subtree rooted at the
templates root and may be symbolic links; the subtree confines where they
can point, whether it is a `cap-std` capability on the working directory
or a view of a Git tree. Links whose target carries a URL scheme, including
single-letter schemes such as `c:`, are not resolved against the tree.

Against a Git-backed source, `bearout generate --check` reads the state
manifest and the existing outputs from that tree, so it verifies what is
staged or committed rather than what is on disk.

`bearout generate --check` reports missing, stale, orphaned, and re-owned
outputs and a stale state manifest. A normal run removes an orphan only when
the state manifest proves Bearout produced that exact path and the file
still carries the recorded digest. Untracked or modified files are never
deleted or overwritten.

Outputs in comment-capable formats carry an SPDX line when the bootstrap
declares a license and always carry a provenance line; the kernel rejects
an output that lacks them. Formats without comments carry provenance only
in the state manifest.

## Why Starlark and JSON Schema

See [technology-evaluation.md](technology-evaluation.md) for the
comparison with Nickel, CUE, and the previous Rhai runtime. In short:
Starlark is deterministic, embeddable in Rust with a contained loader,
linting, static typechecking, cancellation, and execution, heap, and
call-stack limits; JSON Schema 2020-12 is the widely understood declarative
shape layer that editors already support. Nickel could replace both the
bootstrap and the shapes but would overlap with Starlark as a second rule
language; CUE is strong prior art whose reference implementation is Go and
does not fit a single Rust binary.

## Rendering limits

Starlark evaluation is bounded by ticks, heap, and call-stack depth.
Rendering is bounded too: every output renders under `MiniJinja` fuel
(`limits.template_fuel`, measured from the samples with headroom) and into a
writer that stops at `limits.output_bytes` (a conservative bound, not
measured) before an unbounded buffer can be allocated. Exceeding either is
B019 and touches nothing in the tree.

## Trust limitations

Bearout is a capability-confined host with resource limits. It is not a
sandbox for hostile repositories: the limits bound runaway policy code and
the capability confines writes, but checking a repository from an untrusted
author is not a supported security boundary. Contract fixtures do not
change this: they run the same policy over virtual candidates, add no
capability, and write nothing, but a fixture suite from an untrusted
author is as trusted as its policy. History checks read facts from the
repository through the hardened Git runner and write nothing; they do not
verify signatures or the legal truth of a sign-off, and a policy that
enforces commit rules is as trusted as the tree it is read from. See
`SECURITY.md`.
