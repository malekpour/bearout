# SPDX-License-Identifier: Apache-2.0
# Cross-resource invariants of a delivery plan.

load("lib/plan.star", "belonging_to", "contiguous", "of_kind", "total")

def check_allocations_sum_to_budget(project):
    findings = []
    for prj in of_kind(project, "project"):
        allocated = total([a["fields"]["amount"] for a in belonging_to(project, "allocation", prj["id"])])
        if allocated != prj["fields"]["budget"]:
            findings.append(error("allocations total %d budget units, but `budget` is %d" % (allocated, prj["fields"]["budget"]), resource = prj["id"], code = "budget-total"))
        for package in belonging_to(project, "work-package", prj["id"]):
            if len([a for a in belonging_to(project, "allocation", prj["id"]) if a["fields"]["work_package"] == package["id"]]) == 0:
                findings.append(warning("work package has no budget allocation", resource = package["id"], code = "unfunded"))
    return findings

def check_work_packages_are_ordered(project):
    findings = []
    for prj in of_kind(project, "project"):
        findings.extend(contiguous(belonging_to(project, "work-package", prj["id"]), "work package"))
    return findings

def check_milestones_are_ordered_and_dated(project):
    findings = []
    for prj in of_kind(project, "project"):
        milestones = belonging_to(project, "milestone", prj["id"])
        findings.extend(contiguous(milestones, "milestone"))
        previous = ""
        for m in sorted(milestones, key = lambda r: r["fields"]["sequence"]):
            due = m["fields"]["due"]
            if due <= previous:
                findings.append(error("falls due on %s, not after the previous milestone (%s)" % (due, previous), resource = m["id"], code = "chronology"))
            if due < prj["fields"]["starts"] or due > prj["fields"]["ends"]:
                findings.append(error("falls due on %s, outside the project dates %s to %s" % (due, prj["fields"]["starts"], prj["fields"]["ends"]), resource = m["id"], code = "chronology"))
            previous = due
    return findings

def check_roles_are_satisfied(project):
    findings = []
    for prj in of_kind(project, "project"):
        people = belonging_to(project, "participant", prj["id"])
        leads = [p for p in people if p["fields"]["role"] == "lead"]
        if len(leads) != 1:
            findings.append(error("a project needs exactly one lead, found %d" % len(leads), resource = prj["id"], code = "one-lead"))
        lead = project["by_id"][prj["fields"]["lead"]]
        if lead["fields"]["role"] != "lead":
            findings.append(error("`lead` names `%s`, whose role is %s" % (lead["id"], lead["fields"]["role"]), resource = prj["id"], code = "lead-role"))
        if len([p for p in people if p["fields"]["role"] == "reviewer"]) == 0:
            findings.append(error("a project needs at least one reviewer", resource = prj["id"], code = "reviewer"))
        for package in belonging_to(project, "work-package", prj["id"]):
            owner = project["by_id"][package["fields"]["owner"]]
            if owner["fields"]["role"] not in ["engineer", "lead"]:
                findings.append(error("owner `%s` is a %s; work packages are owned by engineers or the lead" % (owner["id"], owner["fields"]["role"]), resource = package["id"], code = "owner-role"))
    return findings

def check_deliverables_belong_to_their_milestone(project):
    findings = []
    for d in of_kind(project, "deliverable"):
        milestone = project["by_id"][d["fields"]["milestone"]]
        if milestone["fields"]["project"] != d["fields"]["project"]:
            findings.append(error("milestone `%s` belongs to a different project" % milestone["id"], resource = d["id"], code = "milestone-project"))
    for m in of_kind(project, "milestone"):
        if len([r for r in m["referenced_by"] if r["field"] == "milestone"]) == 0:
            findings.append(error("a milestone must carry at least one deliverable", resource = m["id"], code = "milestone-empty"))
    return findings
