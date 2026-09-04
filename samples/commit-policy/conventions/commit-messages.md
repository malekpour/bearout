+++
schema = "example/commit-policy/convention@1"
id = "commit-messages"
title = "Commit messages"
+++

# Commit messages

## Rule

Every non-merge commit carries a Conventional Commits header with a type
from this repository's list, a body separated from the header by one
blank line, a `BREAKING CHANGE:` footer when the header carries `!`, and
a `Signed-off-by:` line naming the author of the commit. Merge commits are
exempt; `fixup!` and `squash!` commits are warned about until they are
rebased away. These are this repository's rules, written in
`rules/history.star`; Bearout enforces them without knowing what a type,
a footer, or a sign-off means.
