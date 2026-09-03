# SPDX-License-Identifier: Apache-2.0
# Log-wide invariants. A check receives the whole graph and must name the
# resource each finding is about.

load("lib/records.star", "by_schema", "record_number")

SCHEMA = "example/decision-records/decision@1"

def check_supersession_is_reciprocal(project):
    findings = []
    for record in by_schema(project, SCHEMA):
        for old_id in record["relations"].get("supersedes", []):
            old = project["by_id"][old_id]
            if old["fields"].get("superseded_by") != record["id"]:
                findings.append(error(
                    "`%s` supersedes this record, so `superseded_by` must name it" % record["id"],
                    resource = old_id,
                    code = "supersession-reciprocal",
                ))
        newer_id = record["fields"].get("superseded_by")
        if newer_id != None and record["id"] not in project["by_id"][newer_id]["relations"].get("supersedes", []):
            findings.append(error(
                "`superseded_by` names `%s`, which does not list this record in `supersedes`" % newer_id,
                resource = record["id"],
                code = "supersession-reciprocal",
            ))
    return findings

def check_numbering_is_contiguous(project):
    records = by_schema(project, SCHEMA)
    numbers = sorted([record_number(record["id"]) for record in records])
    findings = []
    for index, number in enumerate(numbers):
        if number != index + 1:
            offender = [r for r in records if record_number(r["id"]) == number][0]
            findings.append(error(
                "record numbers must be contiguous: expected %d, found %d" % (index + 1, number),
                resource = offender["id"],
                code = "numbering",
            ))
            break
    return findings
