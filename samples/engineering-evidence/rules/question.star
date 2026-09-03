# SPDX-License-Identifier: Apache-2.0
# Status must agree with relations, and an unresolved question must say why
# it is open.

load("lib/records.star", "section_titled")

def validate_question(resource):
    f = resource["fields"]
    blocked_by = f.get("blocked_by", [])
    findings = []
    if f["status"] == "blocked" and len(blocked_by) == 0:
        findings.append(error("a blocked question must name what blocks it", code = "blocker-required"))
    if f["status"] != "blocked" and len(blocked_by) > 0:
        findings.append(error("only a blocked question may list `blocked_by`", code = "blocker-premature"))
    if f["status"] == "closed" and "closed_by" not in f:
        findings.append(error("a closed question must name the decision in `closed_by`", code = "closed-by-required"))
    if f["status"] != "closed" and "closed_by" in f:
        findings.append(error("only a closed question may carry `closed_by`", code = "closed-by-premature"))
    if f["status"] != "closed" and section_titled(resource, "Why it is open") == None:
        findings.append(error("an unresolved question must explain `Why it is open`", code = "why-open"))
    return findings
