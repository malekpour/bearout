# SPDX-License-Identifier: Apache-2.0

load("lib/plan.star", "belonging_to", "of_kind", "section_text", "total")

def plan_delivery_documents(project):
    outputs = []
    for prj in of_kind(project, "project"):
        pid = prj["id"]
        people = [{"id": p["id"], "name": p["fields"]["name"], "role": p["fields"]["role"]} for p in belonging_to(project, "participant", pid)]
        allocations = belonging_to(project, "allocation", pid)
        packages = []
        for wp in sorted(belonging_to(project, "work-package", pid), key = lambda r: r["fields"]["sequence"]):
            packages.append({
                "id": wp["id"],
                "sequence": wp["fields"]["sequence"],
                "title": wp["fields"]["title"],
                "owner": project["by_id"][wp["fields"]["owner"]]["fields"]["name"],
                "budget": total([a["fields"]["amount"] for a in allocations if a["fields"]["work_package"] == wp["id"]]),
            })
        milestones = []
        for m in sorted(belonging_to(project, "milestone", pid), key = lambda r: r["fields"]["sequence"]):
            deliverables = [{
                "title": project["by_id"][r["from"]]["fields"]["title"],
                "acceptance_days": project["by_id"][r["from"]]["fields"]["acceptance_days"],
                "criteria": section_text(project["by_id"][r["from"]], "Acceptance criteria"),
            } for r in m["referenced_by"] if r["field"] == "milestone"]
            milestones.append({
                "id": m["id"],
                "sequence": m["fields"]["sequence"],
                "title": m["fields"]["title"],
                "due": m["fields"]["due"],
                "work_package": m["fields"]["work_package"],
                "deliverables": deliverables,
            })
        context = {
            "project": prj["fields"],
            "id": pid,
            "scope": section_text(prj, "Scope"),
            "people": people,
            "packages": packages,
            "milestones": milestones,
        }
        outputs.append(output("delivery-plan.md.j2", "generated/" + pid + "-plan.md", context = context))
        outputs.append(output("schedule.csv.j2", "generated/" + pid + "-schedule.csv", context = context))
    return outputs
