# SPDX-License-Identifier: Apache-2.0
# Shared helpers for decision records, loaded by the validator, the checks,
# and the generator through contained `load()`.

def fragments_of_kind(resource, kind):
    """Fragments of one kind, in document order."""
    return [f for f in resource["fragments"] if f["kind"] == kind]

def section_titled(resource, title):
    """The first section with the given title, or None."""
    for section in resource["sections"]:
        if section["title"] == title:
            return section
    return None

def pad2(n):
    """Two-digit zero-padded text of a small integer."""
    return ("0" + str(n)) if n < 10 else str(n)

def record_number(record_id):
    """The integer in a `decision-NNNN` identifier."""
    return int(record_id.split("-")[1])

def by_schema(project, schema_id):
    """Resources of one schema, in path order."""
    return [project["by_id"][rid] for rid in project["by_schema"].get(schema_id, [])]
