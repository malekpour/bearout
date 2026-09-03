# SPDX-License-Identifier: Apache-2.0
# `uses` must name exactly the symbols the rule text mentions. Existence and
# kind of each symbol are resolved by the kernel from the shape's relation.

def symbols_in(rule):
    out = []
    for word in rule.replace("(", " ").replace(")", " ").replace("[", " ").replace("]", " ").replace("{", " ").replace("}", " ").replace("|", " ").split(" "):
        if word.startswith("token-") or word.startswith("production-"):
            if word not in out:
                out.append(word)
    return out

def validate_production(resource):
    f = resource["fields"]
    mentioned = symbols_in(f["rule"])
    findings = []
    for symbol in f["uses"]:
        if symbol not in mentioned:
            findings.append(error("`uses` names `%s` but the rule never mentions it" % symbol, code = "uses-unmentioned"))
    for symbol in mentioned:
        if symbol not in f["uses"]:
            findings.append(error("the rule mentions `%s` but `uses` omits it" % symbol, code = "uses-incomplete"))
    return findings
