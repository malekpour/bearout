# SPDX-License-Identifier: Apache-2.0
# Per-record rules that need logic. Everything declarative lives in
# decision.schema.toml.

load("lib/records.star", "fragments_of_kind", "pad2", "section_titled")

def validate_decision(resource):
    fields = resource["fields"]
    status = fields["status"]
    rulings = fragments_of_kind(resource, "ruling")
    findings = []

    if status == "accepted" and len(rulings) == 0:
        findings.append(error("an accepted record must carry at least one ruling", code = "rulings-required"))
    if status == "proposed" and len(rulings) > 0:
        findings.append(error("a proposed record must not carry rulings yet", code = "rulings-premature"))

    for index, ruling in enumerate(rulings):
        expected = resource["id"] + "-ruling-" + pad2(index + 1)
        if ruling["id"] != expected:
            findings.append(error(
                "ruling %d must be identified `%s`, found `%s`" % (index + 1, expected, ruling["id"]),
                line = ruling["line"],
                code = "ruling-sequence",
            ))
        section = resource["sections"][ruling["section"]] if ruling["section"] != None else None
        if section == None or section["anchor"] != ruling["id"]:
            findings.append(error(
                "ruling `%s` must sit under a `### %s` heading so it can be cited" % (ruling["id"], ruling["id"]),
                line = ruling["line"],
                code = "ruling-heading",
            ))

    if status == "superseded" and "superseded_by" not in fields:
        findings.append(error("a superseded record must name `superseded_by`", code = "superseded-by-required"))
    if status != "superseded" and "superseded_by" in fields:
        findings.append(error("only a superseded record may name `superseded_by`", code = "superseded-by-premature"))
    if status != "proposed" and section_titled(resource, "Decision") == None:
        findings.append(error("a resolved record must contain a `Decision` section", code = "decision-section"))
    return findings
