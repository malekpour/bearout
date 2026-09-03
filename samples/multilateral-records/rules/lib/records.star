# SPDX-License-Identifier: Apache-2.0

NS = "example/multilateral-records/"

def of_kind(project, kind):
    return [project["by_id"][rid] for rid in project["by_schema"].get(NS + kind + "@1", [])]

def parties_of(project, compact_id):
    return [p for p in of_kind(project, "party") if p["fields"]["compact"] == compact_id]

def section_text(resource, title):
    for section in resource["sections"]:
        if section["title"] == title:
            return section["text"]
    return ""
