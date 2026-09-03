# SPDX-License-Identifier: Apache-2.0

def validate_chapter(resource):
    if resource["fields"]["status"] == "stable":
        return [error("no chapter of Formulo is stable yet", code = "premature-stability")]
    return []
