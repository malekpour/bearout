# SPDX-License-Identifier: Apache-2.0
# Entry module for a fictional contributor handbook assembled from
# versioned sections.

load("section.star", "validate_section")
load("glossary.star", "validate_glossary")
load("assembly.star", "check_handbooks_assemble_current_sections", "check_superseded_sections_are_retired")
load("handbook.star", "plan_handbook")

NS = "example/document-assembly/"

schema(NS + "section@1", shape = "section.schema.toml", validate = validate_section)
schema(NS + "glossary@1", shape = "glossary.schema.toml", validate = validate_glossary)
schema(NS + "handbook@1", shape = "handbook.schema.toml")

check("superseded-sections-are-retired", check_superseded_sections_are_retired)
check("handbooks-assemble-current-sections", check_handbooks_assemble_current_sections)

generator("assembled-handbook", plan_handbook)
