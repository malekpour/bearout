# SPDX-License-Identifier: Apache-2.0

load("lib/records.star", "of_kind", "section_titled")

def plan_registers(project):
    questions = [{
        "id": q["id"],
        "path": q["path"],
        "title": q["fields"]["title"],
        "area": q["fields"]["area"],
        "status": q["fields"]["status"],
        "blocked_by": q["relations"].get("blocked_by", []),
        "closed_by": q["fields"].get("closed_by", ""),
    } for q in of_kind(project, "question")]
    measurements = []
    for m in of_kind(project, "measurement"):
        source = project["by_id"][m["fields"]["source"]]
        used_by = sorted([r["from"] for r in m["referenced_by"] if r["field"] in ["figures", "evidence"]])
        measurements.append({
            "id": m["id"],
            "path": m["path"],
            "quantity": m["fields"]["quantity"],
            "value": m["fields"]["value"],
            "unit": m["fields"]["unit"],
            "method": m["fields"]["method"],
            "source": source["id"],
            "source_kind": source["fields"]["kind"],
            "used_by": used_by,
        })
    decisions = [{
        "id": d["id"],
        "path": d["path"],
        "title": d["fields"]["title"],
        "status": d["fields"]["status"],
        "basis": d["fields"]["basis"],
        "evidence": d["relations"].get("evidence", []),
        "closes": d["relations"].get("closes", []),
    } for d in of_kind(project, "decision")]
    context = {"questions": questions, "measurements": measurements, "decisions": decisions}
    return [
        output("question-register.md.j2", "generated/question-register.md", context = context),
        output("evidence-register.md.j2", "generated/evidence-register.md", context = context),
        output("measurements.csv.j2", "generated/measurements.csv", context = context),
    ]
