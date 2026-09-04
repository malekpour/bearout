# SPDX-License-Identifier: Apache-2.0
# This repository's commit policy. Everything here is a choice of this
# sample: the allowed types, the header shape and length, the body
# separation, the breaking-change footer, the sign-off that must name the
# author, and the exemption of merges and autosquash commits. Bearout
# supplies raw facts (the exact message, the author identity, the parents)
# and a place to report; it holds none of these rules.

# Conventional Commits types this repository accepts.
TYPES = ["build", "chore", "ci", "docs", "feat", "fix", "refactor", "test"]

# The longest header this repository tolerates.
MAX_HEADER = 72

def _header(subject):
    """Split `type(scope)!: summary` into its parts, or return None."""
    head, sep, summary = subject.partition(": ")
    if sep == "":
        return None
    breaking = head.endswith("!")
    if breaking:
        head = head[:-1]
    scope = None
    if head.endswith(")"):
        head, open, rest = head.partition("(")
        if open == "" or rest == ")":
            return None
        scope = rest[:-1]
    return {"type": head, "scope": scope, "breaking": breaking, "summary": summary}

def _sign_off(commit):
    author = commit["author"]
    return "Signed-off-by: %s <%s>" % (author["name"], author["email"])

def _lines(commit):
    return commit["message"].split("\n")

def _has_breaking_footer(commit):
    for line in _lines(commit):
        if line.startswith("BREAKING CHANGE: "):
            return True
    return False

def check_commit_policy(history):
    findings = []
    for commit in history["commits"]:
        key = commit["key"]
        subject = commit["subject"]

        # Merge commits are exempt here; the branch they merge was checked.
        if commit["merge"]:
            continue

        # Autosquash commits are rewritten before they land: a warning, so
        # a pending one is visible but a rebase is not blocked.
        if subject.startswith("fixup! ") or subject.startswith("squash! "):
            findings.append(warning("autosquash commit; rebase before merging", commit = key, line = 1, code = "autosquash"))
            continue

        header = _header(subject)
        if header == None:
            findings.append(error("header must be `<type>(<scope>)!: <summary>`", commit = key, line = 1, code = "header-shape"))
        else:
            if header["type"] not in TYPES:
                findings.append(error("type `%s` is not one of %s" % (header["type"], ", ".join(TYPES)), commit = key, line = 1, code = "header-type"))
            if header["summary"].strip() == "" or header["summary"] != header["summary"].strip():
                findings.append(error("summary must be non-empty without surrounding whitespace", commit = key, line = 1, code = "header-summary"))
            if header["breaking"] and not _has_breaking_footer(commit):
                findings.append(error("a `!` header needs a `BREAKING CHANGE: ` footer", commit = key, code = "breaking-footer"))
        if len(subject) > MAX_HEADER:
            findings.append(error("header is %d characters, above %d" % (len(subject), MAX_HEADER), commit = key, line = 1, code = "header-length"))

        lines = _lines(commit)
        if len(lines) > 1 and lines[1] != "":
            findings.append(error("a body must be separated from the header by one blank line", commit = key, line = 2, code = "body-separation"))

        expected = _sign_off(commit)
        if expected not in lines:
            findings.append(error("missing `%s`" % expected, commit = key, code = "sign-off"))
    return findings
