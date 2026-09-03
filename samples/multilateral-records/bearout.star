# SPDX-License-Identifier: Apache-2.0
# Entry module for the fictional Aurora Research Compact.

load("chronology.star", "check_articles_are_numbered", "check_entry_into_force_is_computed", "check_instruments_follow_the_compact", "check_party_dates_are_consistent")
load("reports.star", "plan_reports")

NS = "example/multilateral-records/"

schema(NS + "compact@1", shape = "compact.schema.toml")
schema(NS + "article@1", shape = "article.schema.toml")
schema(NS + "party@1", shape = "party.schema.toml")
schema(NS + "instrument@1", shape = "instrument.schema.toml")

check("entry-into-force-is-computed", check_entry_into_force_is_computed)
check("party-dates-are-consistent", check_party_dates_are_consistent)
check("articles-are-numbered", check_articles_are_numbered)
check("instruments-follow-the-compact", check_instruments_follow_the_compact)

generator("compact-reports", plan_reports)
