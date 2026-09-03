+++
schema = "example/engineering-evidence/interface@1"
id = "interface-backplane"
title = "Backplane bus"

[[signals]]
name = "clk"
direction = "out"
width = 1

[[signals]]
name = "sync"
direction = "out"
width = 1

[[signals]]
name = "data"
direction = "inout"
width = 8

[[signals]]
name = "present"
direction = "in"
width = 1
+++

# Backplane bus

Synthetic. Directions are given from the core's point of view, named per
[source-signal-naming](../sources/source-signal-naming.md). The physical
layer is open in [question-0001](../questions/question-0001.md).
