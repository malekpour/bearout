# SPDX-License-Identifier: Apache-2.0

NS = "example/project-delivery/"

def of_kind(project, kind):
    return [project["by_id"][rid] for rid in project["by_schema"].get(NS + kind + "@1", [])]

def belonging_to(project, kind, project_id):
    return [r for r in of_kind(project, kind) if r["fields"]["project"] == project_id]

def section_text(resource, title):
    for section in resource["sections"]:
        if section["title"] == title:
            return section["text"]
    return ""

def contiguous(items, label):
    """Findings for sequences that are not 1, 2, 3, ..."""
    ordered = sorted(items, key = lambda r: r["fields"]["sequence"])
    for index, item in enumerate(ordered):
        if item["fields"]["sequence"] != index + 1:
            return [error("%s sequences must be contiguous: expected %d, found %d" % (label, index + 1, item["fields"]["sequence"]), resource = item["id"], code = "sequence")]
    return []

def total(values):
    """Sum of a list of integers; Starlark has no `sum` builtin."""
    result = 0
    for value in values:
        result += value
    return result
