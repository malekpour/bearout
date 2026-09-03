#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Rejects trailing whitespace, a missing final newline, and a blank line at
# the end of every file the candidate working tree would commit: tracked
# files plus untracked files that are not ignored. `git diff --check` only
# sees tracked content, which is wrong before the first commit and blind to
# new files after it.
set -eu
list=$(mktemp)
trap 'rm -f "$list"' EXIT
git ls-files --cached --others --exclude-standard > "$list"
status=0
while IFS= read -r file; do
    [ -f "$file" ] || continue
    if grep -qn '[[:blank:]]$' "$file"; then
        echo "trailing whitespace: $file" >&2
        status=1
    fi
    if [ -s "$file" ]; then
        last=$(tail -c 1 "$file" | od -An -tx1 | tr -d ' \n')
        if [ "$last" != "0a" ]; then
            echo "no newline at end of file: $file" >&2
            status=1
        fi
        last_two=$(tail -c 2 "$file" | od -An -tx1 | tr -d ' \n')
        if [ "$last_two" = "0a0a" ]; then
            echo "blank line at end of file: $file" >&2
            status=1
        fi
    fi
done < "$list"
exit "$status"
