# SPDX-FileCopyrightText: 2026 Livspeil contributors
# SPDX-License-Identifier: MPL-2.0

"""Python SDK for Livspeil scripts.

Scripts run out of process and talk to the kernel over the script worker
protocol (one JSON object per line on stdin/stdout). They read the graph,
cook nodes, and change the document only through commands, which the
kernel validates, journals and can undo::

    import livspeil
    from livspeil import SetInput

    session = livspeil.connect()
    for node in session.graph.nodes(type="core.math.remap"):
        session.graph.apply(SetInput(node, "out_max", 0.5))

What a script may do is decided by the capabilities its host grants
(``graph.read``, ``graph.write``, ``graph.cook``, ``fs.read:<root>``).
"""

import sys as _sys

if _sys.version_info < (3, 10):  # pragma: no cover - guarded at import
    raise ImportError("livspeil needs Python 3.10 or newer")

from .commands import (
    ClearAnimation,
    ClearExpression,
    ClearInput,
    Command,
    CreateNode,
    DeleteNode,
    Link,
    RawCommand,
    Rename,
    SetExpression,
    SetInput,
    Transaction,
    Unlink,
)
from .errors import (
    CapabilityDenied,
    CommandFailed,
    LivspeilError,
    NotFound,
    ProtocolError,
    RemoteError,
    RequestTimeout,
)
from .session import (
    PROTOCOL_VERSION,
    ApplyResult,
    CookResult,
    Event,
    Graph,
    Node,
    Port,
    Session,
    Tick,
    connect,
)
from .values import EVENT, Color, RawValue, Value, Vec2, Vec3, Vec4

__all__ = [
    "EVENT",
    "PROTOCOL_VERSION",
    "ApplyResult",
    "CapabilityDenied",
    "ClearAnimation",
    "ClearExpression",
    "ClearInput",
    "Color",
    "Command",
    "CommandFailed",
    "CookResult",
    "CreateNode",
    "DeleteNode",
    "Event",
    "Graph",
    "Link",
    "LivspeilError",
    "Node",
    "NotFound",
    "Port",
    "ProtocolError",
    "RawCommand",
    "RawValue",
    "RemoteError",
    "Rename",
    "RequestTimeout",
    "Session",
    "SetExpression",
    "SetInput",
    "Tick",
    "Transaction",
    "Unlink",
    "Value",
    "Vec2",
    "Vec3",
    "Vec4",
    "connect",
]
