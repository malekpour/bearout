# SPDX-License-Identifier: Apache-2.0

NS = "example/engineering-evidence/"

def of_kind(project, kind):
    return [project["by_id"][rid] for rid in project["by_schema"].get(NS + kind + "@1", [])]

def section_titled(resource, title):
    for section in resource["sections"]:
        if section["title"] == title:
            return section
    return None

def fragments_of_kind(resource, kind):
    return [f for f in resource["fragments"] if f["kind"] == kind]

def pad2(n):
    return ("0" + str(n)) if n < 10 else str(n)
