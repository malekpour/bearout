# SPDX-License-Identifier: Apache-2.0
# Entry module: one resource schema and one project check over the
# schema-less documents.

load("documents.star", "check_document_links")

schema("example/document-references/topic@1", shape = "topic.schema.toml")
check("document-links-are-descriptive", check_document_links)
