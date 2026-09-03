# SPDX-License-Identifier: Apache-2.0

load("lib/records.star", "fragments_of_kind", "pad2")

def validate_decision(resource):
    rulings = fragments_of_kind(resource, "ruling")
    findings = []
    if resource["fields"]["status"] == "accepted" and len(rulings) == 0:
        findings.append(error("an accepted decision must carry at least one ruling", code = "rulings-required"))
    for index, ruling in enumerate(rulings):
        expected = resource["id"] + "-ruling-" + pad2(index + 1)
        if ruling["id"] != expected:
            findings.append(error("ruling %d must be identified `%s`" % (index + 1, expected), line = ruling["line"], code = "ruling-sequence"))
        section = resource["sections"][ruling["section"]] if ruling["section"] != None else None
        if section == None or section["anchor"] != ruling["id"]:
            findings.append(error("ruling `%s` must sit under a `### %s` heading" % (ruling["id"], ruling["id"]), line = ruling["line"], code = "ruling-heading"))
    return findings
