# SPDX-License-Identifier: Apache-2.0
# Every term fragment sits under a heading named by its id, so it is citable.

def validate_glossary(resource):
    findings = []
    for fragment in resource["fragments"]:
        if fragment["kind"] != "term":
            continue
        section = resource["sections"][fragment["section"]] if fragment["section"] != None else None
        if section == None or section["anchor"] != fragment["id"]:
            findings.append(error("term `%s` must sit under a `### %s` heading" % (fragment["id"], fragment["id"]), line = fragment["line"], code = "term-heading"))
    return findings
