# SPDX-License-Identifier: Apache-2.0
def validate_note(resource):
    if resource["body"].strip() == "":
        return [error("body must not be empty", code = "empty-body")]
    return []
