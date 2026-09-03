# SPDX-License-Identifier: Apache-2.0
# Esperanto collation, defined by this repository. Bearout knows nothing
# about locales; the alphabet order below is the Fundamento's, with the
# circumflex letters following their base letters.

ESPERANTO_ALPHABET = "abcĉdefgĝhĥijĵklmnoprsŝtuŭvz"

def sort_key(text):
    """A list of alphabet positions; unknown characters sort last."""
    key = []
    for ch in text.lower().elems():
        index = ESPERANTO_ALPHABET.find(ch)
        key.append(index if index >= 0 else len(ESPERANTO_ALPHABET))
    return key

def eo_sorted(items, text_of):
    return sorted(items, key = lambda item: sort_key(text_of(item)))

NS = "example/esperanto-reference/"

def of_kind(project, kind):
    return [project["by_id"][rid] for rid in project["by_schema"].get(NS + kind + "@1", [])]

def section_text(resource, title):
    for section in resource["sections"]:
        if section["title"] == title:
            return section["text"]
    return ""
