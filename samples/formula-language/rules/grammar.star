# SPDX-License-Identifier: Apache-2.0
# Whole-grammar invariants.

load("lib/graph.star", "kind_of", "of_kind", "references_from")

def check_sequences_are_contiguous(project):
    chapters = of_kind(project, "chapter")
    numbers = sorted([c["fields"]["sequence"] for c in chapters])
    for index, number in enumerate(numbers):
        if number != index + 1:
            offender = [c for c in chapters if c["fields"]["sequence"] == number][0]
            return [error("chapter sequences must be contiguous: expected %d, found %d" % (index + 1, number), resource = offender["id"], code = "sequence")]
    return []

def check_every_token_is_used(project):
    findings = []
    for token in of_kind(project, "token"):
        if len(references_from(token, "uses")) == 0:
            findings.append(error("token is not used by any production", resource = token["id"], code = "unused-token"))
    return findings

def check_productions_reach_start(project):
    productions = of_kind(project, "production")
    starts = [p for p in productions if p["fields"].get("start", False)]
    if len(starts) != 1:
        return [error("exactly one production must set `start = true`, found %d" % len(starts), resource = p["id"], code = "start") for p in productions[:1]]
    reachable = [starts[0]["id"]]
    queue = [starts[0]["id"]]
    for _ in range(len(productions) + 1):
        if len(queue) == 0:
            break
        current = queue.pop()
        for symbol in project["by_id"][current]["relations"].get("uses", []):
            if kind_of(project, symbol) == "production" and symbol not in reachable:
                reachable.append(symbol)
                queue.append(symbol)
    return [error("production is not reachable from the start production", resource = p["id"], code = "unreachable") for p in productions if p["id"] not in reachable]

def check_operator_precedence_matches_productions(project):
    """Operators sharing a production share a level, and a production that
    contains another has a lower level than it."""
    findings = []
    level = {}
    assoc = {}
    for op in of_kind(project, "operator"):
        if op["fields"]["arity"] != "binary":
            continue
        prod = op["fields"]["production"]
        if prod in level and level[prod] != op["fields"]["precedence"]:
            findings.append(error("precedence %d differs from the other operators of `%s` (%d)" % (op["fields"]["precedence"], prod, level[prod]), resource = op["id"], code = "precedence-level"))
        if prod in assoc and assoc[prod] != op["fields"]["associativity"]:
            findings.append(error("associativity differs from the other operators of `%s`" % prod, resource = op["id"], code = "associativity-level"))
        level[prod] = op["fields"]["precedence"]
        assoc[prod] = op["fields"]["associativity"]
    for prod_id in level:
        for used in project["by_id"][prod_id]["relations"].get("uses", []):
            if used != prod_id and used in level and level[used] <= level[prod_id]:
                findings.append(error("operators of `%s` (level %d) must bind looser than those of `%s` (level %d), which it contains" % (prod_id, level[prod_id], used, level[used]), resource = prod_id, code = "precedence-nesting"))
    return findings

def check_every_error_has_an_example(project):
    return [error("error kind has no example that produces it", resource = e["id"], code = "error-coverage") for e in of_kind(project, "error") if len(references_from(e, "expected_error")) == 0]

def check_every_function_has_an_example(project):
    return [warning("function has no example that calls it", resource = f["id"], code = "function-coverage") for f in of_kind(project, "function") if len(references_from(f, "functions")) == 0]
