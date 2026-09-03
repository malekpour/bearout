# SPDX-License-Identifier: Apache-2.0
# Shared helpers for the Formulo grammar graph.

NS = "example/formula-language/"

def of_kind(project, kind):
    """Resources of one kind, in path order."""
    return [project["by_id"][rid] for rid in project["by_schema"].get(NS + kind + "@1", [])]

def references_from(resource, field):
    """Ids that reference `resource` through `field`."""
    return [r["from"] for r in resource["referenced_by"] if r["field"] == field]

def section_text(resource, title):
    for section in resource["sections"]:
        if section["title"] == title:
            return section["text"]
    return ""

def kind_of(project, rid):
    """The schema kind of a resource id: `token`, `production`, ..."""
    schema_id = project["ids"][rid]
    return schema_id[len(NS):].split("@")[0]
