# SPDX-License-Identifier: Apache-2.0
# Entry module: registers the schemas this project uses. Everything a
# `schema()` call names lives beneath the rules root.

load("note.star", "validate_note")

schema(
    "example/linked-notes/note@1",
    shape = "note.schema.toml",
    validate = validate_note,
)
schema("example/linked-notes/tag@1", shape = "tag.schema.toml")
