# SPDX-License-Identifier: Apache-2.0
load("note.star", "validate_note")

schema("example/fixture/note@1", shape = "note.schema.toml", validate = validate_note)
