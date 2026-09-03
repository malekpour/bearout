# SPDX-License-Identifier: Apache-2.0
# Register-wide invariants.

load("lib/records.star", "of_kind")

def check_closure_is_reciprocal(project):
    findings = []
    for decision in of_kind(project, "decision"):
        for qid in decision["relations"].get("closes", []):
            question = project["by_id"][qid]
            if question["fields"]["status"] != "closed" or question["fields"].get("closed_by") != decision["id"]:
                findings.append(error("`%s` closes this question, so it must be `closed` with `closed_by = \"%s\"`" % (decision["id"], decision["id"]), resource = qid, code = "closure-reciprocal"))
    for question in of_kind(project, "question"):
        closed_by = question["fields"].get("closed_by")
        if closed_by != None and question["id"] not in project["by_id"][closed_by]["relations"].get("closes", []):
            findings.append(error("`closed_by` names `%s`, which does not list this question in `closes`" % closed_by, resource = question["id"], code = "closure-reciprocal"))
    return findings

def check_blocked_only_by_open_questions(project):
    findings = []
    for question in of_kind(project, "question"):
        for blocker in question["relations"].get("blocked_by", []):
            if project["by_id"][blocker]["fields"]["status"] == "closed":
                findings.append(error("blocked by `%s`, which is already closed" % blocker, resource = question["id"], code = "blocked-by-closed"))
    return findings

def check_measurement_basis_cites_measurements(project):
    """The shape already requires `evidence` when the basis is measurement;
    this check adds the converse: an analysis-based decision that cites
    measurements is mislabelled."""
    findings = []
    for decision in of_kind(project, "decision"):
        f = decision["fields"]
        if f["basis"] == "analysis" and len(f.get("evidence", [])) > 0:
            findings.append(warning("cites measurements but states its basis as analysis", resource = decision["id"], code = "basis-mismatch"))
    return findings
