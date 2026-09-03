# SPDX-License-Identifier: Apache-2.0

NS = "example/document-assembly/"
PLACEHOLDERS = ["Project", "Team"]

def of_kind(project, kind):
    return [project["by_id"][rid] for rid in project["by_schema"].get(NS + kind + "@1", [])]

def section_text(resource, title):
    for section in resource["sections"]:
        if section["title"] == title:
            return section["text"]
    return ""

def placeholders_in(text):
    """Every `{Word}` token in `text`, in order of first appearance."""
    found = []
    rest = text
    for _ in range(len(text)):
        start = rest.find("{")
        if start < 0:
            break
        end = rest.find("}", start)
        if end < 0:
            break
        name = rest[start + 1:end]
        if name not in found:
            found.append(name)
        rest = rest[end + 1:]
    return found

def substitute(text, values):
    for name in values:
        text = text.replace("{" + name + "}", values[name])
    return text

def linked_anchors(resource, glossary_file):
    """Fragment anchors of links that point at the glossary file."""
    anchors = []
    for link in resource["links"]:
        target = link["target"]
        if "#" in target and target.split("#")[0].endswith(glossary_file):
            anchors.append(target.split("#")[1])
    return anchors
