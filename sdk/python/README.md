<!--
SPDX-FileCopyrightText: 2026 Livspeil contributors
SPDX-License-Identifier: MPL-2.0
-->

# livspeil (Python SDK)

Client for Livspeil scripts. Scripts run out of process and talk to the
kernel over the script worker protocol (`docs/script-protocol.md` in the
repository): one JSON object per line on stdin/stdout. Pure standard
library, Python 3.10 or newer.

```python
import livspeil
from livspeil import SetInput

session = livspeil.connect()
graph = session.graph
with graph.transaction():
    for node in graph.nodes(type="core.math.remap@1"):
        graph.apply(SetInput(node, "out_max", node.input("out_max") * 0.8))
session.log("done")
```

Run it against a room with
`livspeil run examples/sun-and-lamp.room script.py --grant graph.read,graph.write`.

`print()` output goes to stderr (the protocol owns stdout) and the host
shows it as log lines.

Tests: `python3 -m unittest discover -s tests` in this directory.
