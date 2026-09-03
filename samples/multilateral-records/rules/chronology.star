# SPDX-License-Identifier: Apache-2.0
# Status is a property of the whole set of parties. Under the
# all-signatories rule the compact enters into force on the day the last
# original signatory deposits its ratification, so the recorded `in_force`
# must equal the latest signatory deposit.

load("lib/records.star", "of_kind", "parties_of")

def check_entry_into_force_is_computed(project):
    findings = []
    for compact in of_kind(project, "compact"):
        signatories = [p for p in parties_of(project, compact["id"]) if p["fields"]["status"] == "signatory"]
        if len(signatories) != compact["fields"]["original_signatories"]:
            findings.append(error("records %d original signatories, but %d are on file" % (compact["fields"]["original_signatories"], len(signatories)), resource = compact["id"], code = "signatory-count"))
        last = ""
        last_party = ""
        for p in signatories:
            if p["fields"]["deposited"] > last:
                last = p["fields"]["deposited"]
                last_party = p["id"]
        if last != "" and last != compact["fields"]["in_force"]:
            findings.append(error("under the all-signatories rule the compact entered into force on %s (last deposit: `%s`), but `in_force` says %s" % (last, last_party, compact["fields"]["in_force"]), resource = compact["id"], code = "entry-into-force"))
    return findings

def check_party_dates_are_consistent(project):
    findings = []
    for compact in of_kind(project, "compact"):
        for p in parties_of(project, compact["id"]):
            f = p["fields"]
            if f["status"] == "signatory" and f["signed"] != compact["fields"]["signed"]:
                findings.append(error("an original signatory signed on %s, not %s" % (compact["fields"]["signed"], f["signed"]), resource = p["id"], code = "signed-date"))
            if f["status"] == "acceding" and f["deposited"] <= compact["fields"]["signed"]:
                findings.append(error("an acceding party cannot deposit before the compact was signed", resource = p["id"], code = "accession-date"))
            if "signed" in f and f["deposited"] < f["signed"]:
                findings.append(error("deposited before signing", resource = p["id"], code = "deposit-date"))
            if f["consultative"] and f["consultative_since"] < f["deposited"]:
                findings.append(error("consultative status precedes the deposit of its instrument", resource = p["id"], code = "consultative-date"))
    return findings

def check_articles_are_numbered(project):
    findings = []
    for compact in of_kind(project, "compact"):
        articles = sorted([a for a in of_kind(project, "article") if a["fields"]["compact"] == compact["id"]], key = lambda a: a["fields"]["number"])
        for index, article in enumerate(articles):
            if article["fields"]["number"] != index + 1:
                findings.append(error("articles must be numbered contiguously: expected %d, found %d" % (index + 1, article["fields"]["number"]), resource = article["id"], code = "article-number"))
                break
    return findings

def check_instruments_follow_the_compact(project):
    findings = []
    for instrument in of_kind(project, "instrument"):
        parent = project["by_id"][instrument["fields"]["parent"]]
        if instrument["fields"]["adopted"] <= parent["fields"]["in_force"]:
            findings.append(error("adopted before the parent compact entered into force", resource = instrument["id"], code = "adoption-date"))
        if instrument["fields"]["in_force"] <= instrument["fields"]["adopted"]:
            findings.append(error("entered into force before it was adopted", resource = instrument["id"], code = "instrument-date"))
    return findings
