# SPDX-License-Identifier: Apache-2.0
# Entry module for Formulo. Every language table the generated crate
# contains is derived from these resources.

load("chapter.star", "validate_chapter")
load("token.star", "validate_token")
load("production.star", "validate_production")
load("grammar.star", "check_every_error_has_an_example", "check_every_function_has_an_example", "check_every_token_is_used", "check_operator_precedence_matches_productions", "check_productions_reach_start", "check_sequences_are_contiguous")
load("formulo.star", "plan_formulo")

NS = "example/formula-language/"

schema(NS + "chapter@1", shape = "chapter.schema.toml", validate = validate_chapter)
schema(NS + "token@1", shape = "token.schema.toml", validate = validate_token)
schema(NS + "production@1", shape = "production.schema.toml", validate = validate_production)
schema(NS + "operator@1", shape = "operator.schema.toml")
schema(NS + "function@1", shape = "function.schema.toml")
schema(NS + "error@1", shape = "error.schema.toml")
schema(NS + "example@1", shape = "example.schema.toml")

check("sequences-are-contiguous", check_sequences_are_contiguous)
check("every-token-is-used", check_every_token_is_used)
check("productions-reach-start", check_productions_reach_start)
check("operator-precedence-matches-productions", check_operator_precedence_matches_productions)
check("every-error-has-an-example", check_every_error_has_an_example)
check("every-function-has-an-example", check_every_function_has_an_example)

generator("formulo-crate", plan_formulo)
