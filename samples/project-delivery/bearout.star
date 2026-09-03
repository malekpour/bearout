# SPDX-License-Identifier: Apache-2.0
# Entry module for a fictional project delivery model.

load("delivery.star", "check_allocations_sum_to_budget", "check_deliverables_belong_to_their_milestone", "check_milestones_are_ordered_and_dated", "check_roles_are_satisfied", "check_work_packages_are_ordered")
load("plan.star", "plan_delivery_documents")

NS = "example/project-delivery/"

schema(NS + "project@1", shape = "project.schema.toml")
schema(NS + "participant@1", shape = "participant.schema.toml")
schema(NS + "work-package@1", shape = "work-package.schema.toml")
schema(NS + "milestone@1", shape = "milestone.schema.toml")
schema(NS + "deliverable@1", shape = "deliverable.schema.toml")
schema(NS + "allocation@1", shape = "allocation.schema.toml")

check("allocations-sum-to-budget", check_allocations_sum_to_budget)
check("work-packages-are-ordered", check_work_packages_are_ordered)
check("milestones-are-ordered-and-dated", check_milestones_are_ordered_and_dated)
check("roles-are-satisfied", check_roles_are_satisfied)
check("deliverables-belong-to-their-milestone", check_deliverables_belong_to_their_milestone)

generator("delivery-plan", plan_delivery_documents)
