# SPDX-License-Identifier: Apache-2.0
# A repository rule over schema-less documents: link text must describe its
# target and every image needs alt text. The kernel resolves the links and
# images; what counts as descriptive is this repository's decision.

VAGUE = ["here", "this", "link", "click here"]

def check_document_links(project):
    findings = []
    for document in project["documents"]:
        for link in document["links"]:
            if link["text"].strip().lower() in VAGUE:
                findings.append(error(
                    "link text `%s` does not describe its target `%s`" % (link["text"], link["target"]),
                    path = document["path"],
                    line = link["line"],
                    code = "descriptive-link-text",
                ))
        for image in document["images"]:
            if not image["alt"].strip():
                findings.append(error(
                    "image `%s` needs alt text" % image["target"],
                    path = document["path"],
                    line = image["line"],
                    code = "image-alt-text",
                ))
    return findings
