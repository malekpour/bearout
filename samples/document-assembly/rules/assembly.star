# SPDX-License-Identifier: Apache-2.0

load("lib/text.star", "of_kind")

def check_superseded_sections_are_retired(project):
    findings = []
    for section in of_kind(project, "section"):
        older_id = section["fields"].get("supersedes")
        if older_id != None:
            older = project["by_id"][older_id]
            if older["fields"]["status"] != "retired":
                findings.append(error("superseded by `%s`, so it must be `retired`" % section["id"], resource = older_id, code = "superseded-retired"))
            if older["fields"]["category"] != section["fields"]["category"]:
                findings.append(error("supersedes `%s`, which has a different category" % older_id, resource = section["id"], code = "superseded-category"))
    return findings

def check_handbooks_assemble_current_sections(project):
    superseded = [s["fields"]["supersedes"] for s in of_kind(project, "section") if "supersedes" in s["fields"]]
    findings = []
    for handbook in of_kind(project, "handbook"):
        categories = []
        for sid in handbook["fields"]["sections"]:
            section = project["by_id"][sid]
            if section["fields"]["status"] != "approved":
                findings.append(error("section `%s` is %s, not approved" % (sid, section["fields"]["status"]), resource = handbook["id"], code = "section-approved"))
            if sid in superseded:
                findings.append(error("section `%s` has been superseded by a later version" % sid, resource = handbook["id"], code = "section-current"))
            if section["fields"]["category"] in categories:
                findings.append(error("two sections share the category `%s`" % section["fields"]["category"], resource = handbook["id"], code = "category-unique"))
            categories.append(section["fields"]["category"])
        for category in handbook["fields"]["required_categories"]:
            if category not in categories:
                findings.append(error("a handbook must include a `%s` section" % category, resource = handbook["id"], code = "category-required"))
    return findings
