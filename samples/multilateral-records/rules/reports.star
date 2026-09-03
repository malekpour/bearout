# SPDX-License-Identifier: Apache-2.0

load("lib/records.star", "of_kind", "parties_of", "section_text")

def plan_reports(project):
    outputs = []
    for compact in of_kind(project, "compact"):
        cid = compact["id"]
        parties = sorted([p["fields"] for p in parties_of(project, cid)], key = lambda f: (f["deposited"], f["name"]))
        articles = sorted([{"number": a["fields"]["number"], "title": a["fields"]["title"], "summary": section_text(a, "Summary")} for a in of_kind(project, "article") if a["fields"]["compact"] == cid], key = lambda a: a["number"])
        instruments = sorted([i["fields"] for i in of_kind(project, "instrument") if i["fields"]["parent"] == cid], key = lambda f: f["adopted"])
        context = {
            "compact": compact["fields"],
            "summary": section_text(compact, "Summary"),
            "parties": parties,
            "articles": articles,
            "instruments": instruments,
        }
        outputs.append(output("overview.md.j2", "generated/" + cid + "-overview.md", context = context))
        outputs.append(output("parties.md.j2", "generated/" + cid + "-parties.md", context = context))
        outputs.append(output("parties.csv.j2", "generated/" + cid + "-parties.csv", context = context))
    return outputs
