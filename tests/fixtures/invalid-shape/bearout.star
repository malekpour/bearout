# SPDX-License-Identifier: Apache-2.0
def validate_note(resource):
    return [error("validator ran on " + resource["id"], code = "ran")]

def check_all(project):
    return [error("check ran", resource = rid, code = "ran") for rid in project["by_id"]]

schema("example/fixture/note@1", shape = "note.schema.toml", validate = validate_note)
check("check-ran", check_all)
