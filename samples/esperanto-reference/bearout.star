# SPDX-License-Identifier: Apache-2.0
# Entry module for a small, sourced Esperanto reference.

load("term.star", "validate_term")
load("reference.star", "check_chapters_are_contiguous", "check_every_morpheme_is_used", "check_examples_cite_rules", "check_rules_cite_sources")
load("outputs.star", "plan_reference_outputs")

NS = "example/esperanto-reference/"

schema(NS + "chapter@1", shape = "chapter.schema.toml")
schema(NS + "rule@1", shape = "rule.schema.toml")
schema(NS + "morpheme@1", shape = "morpheme.schema.toml")
schema(NS + "term@1", shape = "term.schema.toml", validate = validate_term)
schema(NS + "example@1", shape = "example.schema.toml")
schema(NS + "source@1", shape = "source.schema.toml")

check("chapters-are-contiguous", check_chapters_are_contiguous)
check("rules-cite-sources", check_rules_cite_sources)
check("examples-cite-rules", check_examples_cite_rules)
check("every-morpheme-is-used", check_every_morpheme_is_used)

generator("reference-outputs", plan_reference_outputs)
