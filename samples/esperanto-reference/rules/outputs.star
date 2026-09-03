# SPDX-License-Identifier: Apache-2.0

load("lib/eo.star", "eo_sorted", "of_kind", "section_text")

def plan_reference_outputs(project):
    sources = {s["id"]: s for s in of_kind(project, "source")}
    rules = sorted(of_kind(project, "rule"), key = lambda r: r["fields"]["number"])
    chapters = []
    for chapter in sorted(of_kind(project, "chapter"), key = lambda c: c["fields"]["sequence"]):
        chapter_rules = [r for r in rules if r["fields"].get("chapter") == chapter["id"]]
        chapters.append({
            "title": chapter["fields"]["title"],
            "sequence": chapter["fields"]["sequence"],
            "celo": section_text(chapter, "Celo"),
            "purpose": section_text(chapter, "Purpose"),
            "rules": [{
                "id": r["id"],
                "number": r["fields"]["number"],
                "title": r["fields"]["title"],
                "regulo": section_text(r, "Regulo"),
                "rule": section_text(r, "Rule"),
                "source": sources[r["fields"]["source"]]["fields"]["title"],
                "url": sources[r["fields"]["source"]]["fields"]["url"],
                "locator": r["fields"]["locator"],
            } for r in chapter_rules],
        })
    morphemes = [{
        "id": m["id"],
        "form": m["fields"]["form"],
        "kind": m["fields"]["kind"],
        "meaning": m["fields"]["meaning"],
        "rule": m["fields"]["rule"],
        "terms": [project["by_id"][r["from"]]["fields"]["esperanto"] for r in m["referenced_by"] if r["field"] == "morphemes"],
    } for m in of_kind(project, "morpheme")]
    morphemes = eo_sorted(morphemes, lambda m: m["form"].strip("-"))
    terms = eo_sorted([{
        "id": t["id"],
        "esperanto": t["fields"]["esperanto"],
        "english": t["fields"]["english"],
        "part_of_speech": t["fields"]["part_of_speech"],
        "morphemes": t["fields"]["morphemes"],
        "attested_in": t["fields"]["attested_in"],
    } for t in of_kind(project, "term")], lambda t: t["esperanto"])
    examples = [{
        "id": e["id"],
        "esperanto": e["fields"]["esperanto"],
        "english": e["fields"]["english"],
        "rules": e["fields"]["rules"],
        "source": e["fields"]["source"],
        "locator": e["fields"]["locator"],
    } for e in of_kind(project, "example")]
    context = {"chapters": chapters, "morphemes": morphemes, "terms": terms, "examples": examples, "sources": [s["fields"] for s in of_kind(project, "source")]}
    return [
        output("grammar-reference.md.j2", "generated/grammar-reference.md", context = context),
        output("morpheme-index.md.j2", "generated/morpheme-index.md", context = context),
        output("glossary.json.j2", "generated/glossary.json", context = context),
        output("examples.json.j2", "generated/examples.json", context = context),
    ]
