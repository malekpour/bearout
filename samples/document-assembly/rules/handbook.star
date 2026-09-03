# SPDX-License-Identifier: Apache-2.0
# Assemble each handbook: number its sections, substitute placeholders,
# and select the glossary terms those sections use.

load("lib/text.star", "of_kind", "section_text", "substitute")

def plan_handbook(project):
    outputs = []
    for handbook in of_kind(project, "handbook"):
        f = handbook["fields"]
        values = {"Project": f["project_name"], "Team": f["team_name"]}
        sections = []
        used = []
        for index, sid in enumerate(f["sections"]):
            section = project["by_id"][sid]
            sections.append({
                "number": index + 1,
                "id": sid,
                "title": section["fields"]["title"],
                "version": section["fields"]["version"],
                "category": section["fields"]["category"],
                "text": substitute(section_text(section, "Text"), values),
            })
            for term in section["relations"].get("uses_terms", []):
                if term not in used:
                    used.append(term)
        glossary = project["by_id"][f["glossary"]]
        terms = [{"id": t["id"], "term": t["fields"]["term"], "text": t["fields"]["text"]} for t in glossary["fragments"] if t["kind"] == "term" and t["id"] in used]
        context = {
            "handbook": f,
            "introduction": substitute(section_text(handbook, "Introduction"), values),
            "sections": sections,
            "terms": terms,
        }
        outputs.append(output("handbook.md.j2", "generated/" + handbook["id"] + ".md", context = context))
        outputs.append(output("glossary.json.j2", "generated/" + handbook["id"] + "-glossary.json", context = context))
    return outputs
