# Sample: commit-policy

## Purpose

A repository-owned commit policy checked by `bearout history`: the rules
about commit messages that many repositories keep in a script live here
in Starlark, over the exact facts Bearout captures from Git, and are
regression-tested without any Git repository through pending-message
fixture cases.

## Data classification

Synthetic. The one convention record documents the policy itself.

## Capabilities demonstrated

- **History policy registration.** `bearout.star` registers
  `history_check("commit-policy", check_commit_policy)`; the check runs
  only for `bearout history range`, `bearout history message`, and the
  history fixture cases, never for `check`, `generate`, or `format`.
- **Sample policy choices, not kernel rules.** `rules/history.star`
  decides everything: the accepted Conventional Commits types, the
  `type(scope)!: summary` header shape and its 72-character bound, the
  blank line before a body, the `BREAKING CHANGE:` footer a `!` header
  needs, the `Signed-off-by:` line that must name the commit's author,
  the exemption of merge commits, and the warning on `fixup!` and
  `squash!` commits. Bearout knows none of these words.
- **Raw facts.** The policy reads `commit["subject"]`,
  `commit["message"]` split on newlines, `commit["author"]`, and
  `commit["merge"]`, and targets `commit["key"]` with a message line.
- **Pending-message fixtures.** `contract-tests/messages.test.toml`
  supplies synthetic commit-msg inputs with `[cases.history]` and expects
  B032 and B033 findings by `commit`, `line`, and `rule`; the suite is
  hermetic and runs no Git command.

## Resource model

| Schema | Fields | Fragments |
| --- | --- | --- |
| `example/commit-policy/convention@1` | `title` | none |

Records: `commit-messages`, the convention the policy enforces.

## Generated artifacts

None.

## Try breaking it

- Run `bearout test samples/commit-policy`: nine cases pass. Change
  `MAX_HEADER` to 20 in `rules/history.star`: the first case fails with
  an unexpected `header-length` finding, exit code 1.
- Remove `"feat"` from `TYPES`: two cases fail.
- Commit the sample into a repository of its own and run
  `bearout history range --base HEAD~1 samples/commit-policy` after a
  commit whose subject lacks a type: B032 `header-shape` on that commit,
  exit code 1. Install a `commit-msg` hook that runs
  `bearout history message samples/commit-policy --file "$1"` and the
  same finding stops the commit.

## Sample omissions

No scope allow-list, no changed-path rules, no range-wide rules such as a
maximum commit count, and no committer checks.

## Engine gaps

- **No range fixtures.** Range topology and changed-path policy are
  covered by Bearout's own synthetic-repository tests; a declarative
  history DAG fixture is deferred until a real consumer needs one.
- **No signature verification and no mailmap.** The policy sees raw
  identities and cannot verify a GPG or SSH signature or the legal truth
  of a sign-off.
