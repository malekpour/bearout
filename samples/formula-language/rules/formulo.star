# SPDX-License-Identifier: Apache-2.0
# Plans the generated crate, the syntax reference, and the conformance
# corpus. Every table in the generated Rust is built here from the graph:
# tokens, keywords, operators with their precedence and associativity, the
# function registry, error kinds, and examples.

load("lib/graph.star", "kind_of", "of_kind", "section_text")

def _variant(name):
    return "".join([part.capitalize() for part in name.split("-")])

def _summary(resource):
    return section_text(resource, "Summary").replace("\n", " ")

def plan_formulo(project):
    tokens = of_kind(project, "token")
    fixed = sorted(
        [{"variant": t["fields"]["variant"], "text": t["fields"]["text"], "id": t["id"]} for t in tokens if t["fields"]["kind"] == "punctuator"],
        key = lambda t: (-len(t["text"]), t["text"]),
    )
    keywords = [{"variant": t["fields"]["variant"], "text": t["fields"]["text"], "id": t["id"]} for t in tokens if t["fields"]["kind"] == "keyword"]
    open_tokens = [{"variant": t["fields"]["variant"], "pattern": t["fields"]["pattern"], "id": t["id"], "kind": t["fields"]["kind"]} for t in tokens if "pattern" in t["fields"]]

    operators = of_kind(project, "operator")
    binary = []
    unary = []
    for op in operators:
        f = op["fields"]
        entry = {
            "id": op["id"],
            "name": f["name"],
            "variant": _variant(f["name"]),
            "symbol": f["symbol"],
            "precedence": f["precedence"],
            "right_associative": f["associativity"] == "right",
            "associativity": f["associativity"],
            "token_variant": project["by_id"][f["token"]]["fields"]["variant"],
            "production": f["production"],
        }
        if f["arity"] == "binary":
            binary.append(entry)
        else:
            unary.append(entry)

    functions = [{
        "id": fn["id"],
        "name": fn["fields"]["name"],
        "min_args": fn["fields"]["min_args"],
        "max_args": fn["fields"].get("max_args"),
        "summary": _summary(fn),
    } for fn in of_kind(project, "function")]

    errors = [{"id": e["id"], "variant": e["fields"]["variant"], "summary": _summary(e)} for e in of_kind(project, "error")]

    examples = []
    for ex in of_kind(project, "example"):
        f = ex["fields"]
        examples.append({
            "id": ex["id"],
            "test_name": ex["id"].replace("-", "_"),
            "source": f["source"],
            "expected_ast": f.get("expected_ast"),
            # rustfmt keeps `assert_eq!(expr.to_string(), "...");` on one line only when its arguments fit in fn_call_width (60).
            "inline": len(f.get("expected_ast", "")) + 20 <= 60,
            "expected_error": f.get("expected_error"),
            "expected_error_variant": project["by_id"][f["expected_error"]]["fields"]["variant"] if "expected_error" in f else None,
            "note": ex["body"].split("\n\n")[-1].strip().replace("\n", " ") if ex["body"].strip() != "" else "",
        })

    start = [p for p in of_kind(project, "production") if p["fields"].get("start", False)][0]
    lead_token = [s for s in start["relations"]["uses"] if kind_of(project, s) == "token"][0]

    context = {
        "chapters": sorted(
            [{"id": c["id"], "title": c["fields"]["title"], "sequence": c["fields"]["sequence"], "purpose": section_text(c, "Purpose")} for c in of_kind(project, "chapter")],
            key = lambda c: c["sequence"],
        ),
        "tokens": [{"id": t["id"], "kind": t["fields"]["kind"], "variant": t["fields"]["variant"], "text": t["fields"].get("text"), "pattern": t["fields"].get("pattern")} for t in tokens],
        "fixed": fixed,
        "keywords": keywords,
        "keywords_inline": len(", ".join(["(\"%s\", Token::%s)" % (k["text"], k["variant"]) for k in keywords])) < 55,
        "open_tokens": open_tokens,
        "productions": [{"id": p["id"], "rule": p["fields"]["rule"], "start": p["fields"].get("start", False)} for p in of_kind(project, "production")],
        "binary": binary,
        "unary": unary,
        "functions": functions,
        "errors": errors,
        "examples": examples,
        "lead_token_variant": project["by_id"][lead_token]["fields"]["variant"],
    }
    return [
        output("ast.rs.j2", "formulo/src/generated/ast.rs", context = context),
        output("lexer.rs.j2", "formulo/src/generated/lexer.rs", context = context),
        output("parser.rs.j2", "formulo/src/generated/parser.rs", context = context),
        output("conformance.rs.j2", "formulo/tests/conformance.rs", context = context),
        output("syntax-reference.md.j2", "generated/syntax-reference.md", context = context),
        output("conformance.json.j2", "generated/conformance.json", context = context),
    ]
