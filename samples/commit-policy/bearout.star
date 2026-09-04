# SPDX-License-Identifier: Apache-2.0
# Entry module: the convention schema and the one history check. Every rule
# about commit messages lives in rules/history.star; the kernel supplies
# the facts and runs the check for `bearout history` and the fixture suite.

load("history.star", "check_commit_policy")

schema("example/commit-policy/convention@1", shape = "convention.schema.toml")
history_check("commit-policy", check_commit_policy)
