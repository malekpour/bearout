# SPDX-License-Identifier: Apache-2.0
# Plans the decision index. The generator decides what to render; the
# kernel renders it and delivers it beneath the declared output root.

load("lib/records.star", "by_schema", "fragments_of_kind")

def plan_decision_index(project):
    records = []
    for record in by_schema(project, "example/decision-records/decision@1"):
        records.append({
            "id": record["id"],
            "path": record["path"],
            "title": record["fields"]["title"],
            "status": record["fields"]["status"],
            "date": record["fields"]["date"],
            "rulings": [{"id": r["id"], "text": r["fields"]["text"]} for r in fragments_of_kind(record, "ruling")],
        })
    return [output("decision-index.md.j2", "generated/decision-index.md", context = {"records": records})]
