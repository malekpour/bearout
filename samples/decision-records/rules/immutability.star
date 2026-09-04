# SPDX-License-Identifier: Apache-2.0
# What immutability means for this log. Bearout supplies the baseline view
# and the change facts; this repository decides which records are
# protected, what may still change, and what a deletion or a move means.
# Without a baseline the check is inactive.

load("lib/records.star", "by_schema", "fragments_of_kind", "section_titled")

SCHEMA = "example/decision-records/decision@1"

# A record is protected once it has been resolved.
PROTECTED_STATUSES = ["accepted", "rejected", "superseded"]

# The only status change a protected record may undergo.
PERMITTED_TRANSITIONS = [("accepted", "superseded")]

def _rulings(record):
    return [(r["id"], r["fields"]["text"]) for r in fragments_of_kind(record, "ruling")]

def _decision_text(record):
    section = section_titled(record, "Decision")
    return section["text"] if section != None else None

def check_protected_records(project):
    comparison = project["comparison"]
    if comparison == None:
        return []
    baseline = comparison["baseline"]
    findings = []
    for old in by_schema(baseline, SCHEMA):
        if old["fields"]["status"] not in PROTECTED_STATUSES:
            continue
        new = project["by_id"].get(old["id"])
        if new == None:
            findings.append(error(
                "protected record `%s` was deleted; supersede it with a new record instead" % old["id"],
                resource = old["id"],
                side = "baseline",
                code = "protected-record-deleted",
            ))
            continue
        if new["path"] != old["path"]:
            findings.append(warning(
                "protected record moved from `%s`" % old["path"],
                resource = old["id"],
                code = "protected-record-moved",
            ))
        old_status = old["fields"]["status"]
        new_status = new["fields"]["status"]
        if old_status != new_status and (old_status, new_status) not in PERMITTED_TRANSITIONS:
            findings.append(error(
                "protected record changed status from `%s` to `%s`" % (old_status, new_status),
                resource = old["id"],
                code = "protected-status",
            ))
        if old["fields"]["date"] != new["fields"]["date"]:
            findings.append(error(
                "protected record changed its date; only `title`, relations, and the Context section may be corrected",
                resource = old["id"],
                code = "protected-field",
            ))
        if _rulings(old) != _rulings(new):
            findings.append(error(
                "protected record changed its rulings; a ruling changes only through a superseding record",
                resource = old["id"],
                code = "protected-rulings",
            ))
        if _decision_text(old) != _decision_text(new):
            findings.append(error(
                "protected record changed its Decision section",
                resource = old["id"],
                code = "protected-decision",
            ))
    current = [d["path"] for d in project["documents"]]
    for document in baseline["documents"]:
        if document["path"] not in current:
            findings.append(error(
                "document `%s` was removed from the selection" % document["path"],
                path = document["path"],
                side = "baseline",
                code = "document-removed",
            ))
    return findings
