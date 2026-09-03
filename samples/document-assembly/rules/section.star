# SPDX-License-Identifier: Apache-2.0
# Placeholders are validated before anything is rendered, and terminology
# is checked through explicit links to the glossary, not by substring
# matching: the declared `uses_terms` must equal the glossary anchors the
# text links to.

load("lib/text.star", "PLACEHOLDERS", "linked_anchors", "placeholders_in", "section_text")

def validate_section(resource):
    f = resource["fields"]
    text = section_text(resource, "Text")
    findings = []
    if text.strip() == "":
        findings.append(error("the Text section must not be empty", code = "empty-text"))
    for name in placeholders_in(text):
        if name not in PLACEHOLDERS:
            findings.append(error("unknown placeholder `{%s}`; only %s are substituted" % (name, PLACEHOLDERS), code = "unknown-placeholder"))
    if "supersedes" in f and f["version"] < 2:
        findings.append(error("a section that supersedes another must be version 2 or later", code = "supersedes-version"))
    declared = f.get("uses_terms", [])
    linked = linked_anchors(resource, "glossary-core.md")
    for term in declared:
        if term not in linked:
            findings.append(error("declares `%s` but the text never links to it" % term, code = "term-unlinked"))
    for anchor in linked:
        if anchor not in declared:
            findings.append(error("links to glossary anchor `%s` without declaring it in `uses_terms`" % anchor, code = "term-undeclared"))
    return findings
