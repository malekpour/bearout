#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Enforces the commit conventions in CONTRIBUTING.md: a Conventional Commits
# header and a Developer Certificate of Origin sign-off. Dependency-free so
# that the hook runs wherever git does.
set -eu
file="$1"
header=$(sed -n '1p' "$file")
if ! printf '%s\n' "$header" | grep -Eq '^(build|chore|ci|docs|feat|fix|perf|refactor|revert|style|test)(\([a-z0-9./-]+\))?!?: [^ ].*$'; then
    echo "commit header must follow Conventional Commits, e.g. 'feat(graph): resolve typed relations'" >&2
    echo "found: $header" >&2
    exit 1
fi
if ! grep -Eq '^Signed-off-by: .+ <.+@.+>$' "$file"; then
    echo "commit message needs a DCO sign-off; use 'git commit -s'" >&2
    exit 1
fi
