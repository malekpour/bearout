# SPDX-License-Identifier: Apache-2.0
# A fixed token carries `text`; an open-ended token carries `pattern`.

def validate_token(resource):
    f = resource["fields"]
    fixed = f["kind"] in ["punctuator", "keyword"]
    findings = []
    if fixed and "text" not in f:
        findings.append(error("a %s token must carry `text`" % f["kind"], code = "token-text"))
    if not fixed and "pattern" not in f:
        findings.append(error("a %s token must carry `pattern`" % f["kind"], code = "token-pattern"))
    if "text" in f and "pattern" in f:
        findings.append(error("a token carries either `text` or `pattern`, not both", code = "token-shape"))
    return findings
