# SPDX-License-Identifier: Apache-2.0

def validate_interface(resource):
    seen = []
    findings = []
    for signal in resource["fields"]["signals"]:
        if signal["name"] in seen:
            findings.append(error("signal `%s` is declared twice" % signal["name"], code = "duplicate-signal"))
        seen.append(signal["name"])
    return findings
