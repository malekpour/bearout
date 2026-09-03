# SPDX-License-Identifier: Apache-2.0
# The one rule that needs logic: the Summary section must say something.
# Types, required fields, the required section, and every relation are
# declared in note.schema.toml and checked by the kernel before this runs.

def validate_note(resource):
    findings = []
    for section in resource["sections"]:
        if section["title"] == "Summary" and section["text"].strip() == "":
            findings.append(error("the Summary section must not be empty", line = section["line"], code = "empty-summary"))
    return findings
