# SPDX-License-Identifier: Apache-2.0

load("lib/eo.star", "of_kind")

def check_chapters_are_contiguous(project):
    chapters = sorted(of_kind(project, "chapter"), key = lambda c: c["fields"]["sequence"])
    for index, chapter in enumerate(chapters):
        if chapter["fields"]["sequence"] != index + 1:
            return [error("chapter sequences must be contiguous: expected %d, found %d" % (index + 1, chapter["fields"]["sequence"]), resource = chapter["id"], code = "sequence")]
    return []

def check_rules_cite_sources(project):
    findings = []
    for rule in of_kind(project, "rule"):
        source = project["by_id"][rule["fields"]["source"]]
        if not source["fields"]["url"].startswith("https://www.akademio-de-esperanto.org/"):
            findings.append(warning("cites a source outside the Akademio de Esperanto", resource = rule["id"], code = "source-authority"))
    return findings

def check_examples_cite_rules(project):
    findings = []
    for rule in of_kind(project, "rule"):
        if len([r for r in rule["referenced_by"] if r["field"] == "rules"]) == 0:
            findings.append(warning("no example illustrates this rule", resource = rule["id"], code = "rule-coverage"))
    return findings

def check_every_morpheme_is_used(project):
    findings = []
    for morpheme in of_kind(project, "morpheme"):
        if len([r for r in morpheme["referenced_by"] if r["field"] == "morphemes"]) == 0:
            findings.append(warning("no term uses this morpheme", resource = morpheme["id"], code = "morpheme-coverage"))
    return findings
