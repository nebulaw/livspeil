# SPDX-FileCopyrightText: 2026 Livspeil contributors
# SPDX-License-Identifier: MPL-2.0

"""Sessions, the graph API and hooks."""

from __future__ import annotations

import contextlib
import os
import re
import sys
from dataclasses import dataclass, field
from typing import Any, Callable, Iterator, Optional, TextIO, TypeVar

from .commands import Command, CreateNode, DeleteNode, NodeRef, Rename, Transaction, walk
from .errors import ProtocolError, RemoteError
from .transport import Connection
from .values import Value, decode

PROTOCOL_VERSION = 1
"""The protocol version this SDK speaks."""

CLIENT = "livspeil-python/0.0.0"

_UUID = re.compile(r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$")


@dataclass(frozen=True)
class Tick:
    """A frame tick: frame counter, time and delta in seconds, and how many
    scene events fired in it."""

    frame: int
    time: float
    dt: float
    fired: int = 0


@dataclass(frozen=True)
class Event:
    """A host event. ``kind`` is ``"scene"`` for an event fired by a node
    output (``node``, ``port``, ``payload`` set) or another kind sent by
    the host (``data`` holds the whole message)."""

    kind: str
    node: Optional[str] = None
    port: Optional[str] = None
    payload: Optional[Value] = None
    data: dict[str, Any] = field(default_factory=dict)


@dataclass(frozen=True)
class Port:
    """One input or output of a node, as last read from the host."""

    name: str
    type: str
    value: Optional[Value] = None
    default: Optional[Value] = None
    link: Optional[str] = None
    expression: Optional[str] = None
    animated: bool = False

    @property
    def effective(self) -> Optional[Value]:
        """The static value if set, else the default (links, expressions
        and animation are not evaluated; cook for that)."""
        return self.value if self.value is not None else self.default


@dataclass(frozen=True)
class ApplyResult:
    """What :meth:`Graph.apply` did: the document changes, and the depth
    of the undo stack after it."""

    changes: list[dict[str, Any]]
    undo_len: int


@dataclass(frozen=True)
class CookResult:
    """Outputs of one cooked node, by port name, and its error, if any (a
    failed node may still report its last good outputs)."""

    node: str
    outputs: dict[str, Optional[Value]]
    error: Optional[str] = None

    def __getitem__(self, port: str) -> Optional[Value]:
        return self.outputs[port]


class Node:
    """A node of the graph. ``id``, ``name``, ``path`` and ``type`` are
    known from listing; ports are fetched on first use (see
    :meth:`refresh`)."""

    def __init__(self, graph: "Graph", info: dict[str, Any]) -> None:
        self._graph = graph
        self.id: str = info["id"]
        self.name: str = info.get("name") or ""
        self.path: str = info.get("path") or ""
        self.type: str = info.get("type") or ""
        self.parent: Optional[str] = info.get("parent")
        self.subnet: bool = bool(info.get("subnet"))
        self._inputs: Optional[dict[str, Port]] = None
        self._outputs: Optional[dict[str, Port]] = None
        if "inputs" in info:
            self._load(info)

    def __repr__(self) -> str:
        return f"Node({self.path!r}, type={self.type!r})"

    def _load(self, info: dict[str, Any]) -> None:
        def port(p: dict[str, Any]) -> Port:
            link = p.get("link")
            return Port(
                name=p["name"],
                type=str(p.get("type")),
                value=decode(p.get("value")),
                default=decode(p.get("default")),
                link=f"{link['node']}.{link['port']}" if link else None,
                expression=p.get("expression"),
                animated=bool(p.get("animated")),
            )

        self._inputs = {p["name"]: port(p) for p in info.get("inputs", [])}
        self._outputs = {p["name"]: port(p) for p in info.get("outputs", [])}

    def refresh(self) -> "Node":
        """Re-read the node's ports from the host (``graph.read``)."""
        self._load(self._graph._request("graph.node", {"node": self.id}))
        return self

    @property
    def inputs(self) -> dict[str, Port]:
        """The node's inputs by name."""
        if self._inputs is None:
            self.refresh()
        return self._inputs or {}

    @property
    def outputs(self) -> dict[str, Port]:
        """The node's outputs by name."""
        if self._outputs is None:
            self.refresh()
        return self._outputs or {}

    def input(self, name: str) -> Optional[Value]:
        """The static value of input ``name``, or its default when unset.

        Raises:
            KeyError: the node has no such input.
        """
        return self.inputs[name].effective


class Graph:
    """The document's graph, seen through the protocol."""

    def __init__(self, session: "Session") -> None:
        self._session = session
        self._pending: Optional[list[Command]] = None
        self._ids: dict[str, str] = {}

    def _request(self, method: str, params: Any = None) -> Any:
        return self._session.request(method, params)

    def nodes(
        self,
        network: Optional[str] = None,
        recursive: bool = True,
        type: Optional[str] = None,  # noqa: A002 - matches the wire name
    ) -> list[Node]:
        """Nodes of the root network (or of subnet ``network``), with the
        nodes inside subnets when ``recursive``, sorted by path. ``type``
        keeps only nodes of that type: qualified (``core.math.add@1``) or
        any version (``core.math.add``). Needs ``graph.read``."""
        params: dict[str, Any] = {"recursive": recursive}
        if network is not None:
            params["network"] = network
        found = [Node(self, n) for n in self._request("graph.nodes", params)["nodes"]]
        for node in found:
            self._ids[node.path] = node.id
        if type is not None:
            found = [n for n in found if n.type == type or n.type.split("@")[0] == type]
        return found

    def node(self, ref: NodeRef) -> Node:
        """One node with its ports, by path or id. Needs ``graph.read``.

        Raises:
            NotFound: no such node.
        """
        key = ref.id if isinstance(ref, Node) else ref
        return Node(self, self._request("graph.node", {"node": key}))

    def snapshot(self) -> dict[str, Any]:
        """The whole editable state in the kernel's snapshot form, plus a
        ``paths`` map from id to path. Needs ``graph.read``."""
        result: dict[str, Any] = self._request("graph.snapshot")
        return result

    def new_ids(self, count: int = 1) -> list[str]:
        """Fresh node ids from the host. Needs ``graph.write``."""
        ids: list[str] = self._request("graph.new_id", {"count": count})["ids"]
        return ids

    def resolve(self, ref: NodeRef) -> str:
        """The id of a node reference (ids pass through; paths are looked
        up once and cached)."""
        if isinstance(ref, Node):
            return ref.id
        if _UUID.match(ref):
            return ref
        path = ref if ref.startswith("/") else "/" + ref
        if path not in self._ids:
            self._ids[path] = self.node(ref).id
        return self._ids[path]

    def apply(self, command: Command, group: Optional[int] = None) -> Optional[ApplyResult]:
        """Apply a command through the kernel's command bus (validated,
        undoable, journaled with origin ``script``). Needs ``graph.write``.

        Inside :meth:`transaction` the command is queued instead and
        ``None`` is returned. ``group``: consecutive applies with the same
        group become one undo step.

        Raises:
            CommandFailed: the command was rejected; nothing changed.
            CapabilityDenied: ``graph.write`` is not granted.
        """
        for step in walk(command):
            if isinstance(step, CreateNode) and step.id is None:
                step.id = self.new_ids(1)[0]
        if self._pending is not None:
            self._pending.append(command)
            return None
        params: dict[str, Any] = {"command": command.to_json(self.resolve)}
        if group is not None:
            params["group"] = group
        result = self._request("graph.apply", params)
        for step in walk(command):
            if isinstance(step, (CreateNode, DeleteNode, Rename)):
                self._ids.clear()
                break
        return ApplyResult(changes=list(result.get("changes", [])), undo_len=int(result["undo_len"]))

    @contextlib.contextmanager
    def transaction(self) -> Iterator[list[Command]]:
        """Collect the commands applied in the block and apply them as one
        transaction (all or nothing, one undo step) when it ends without
        an exception. Yields the list of queued commands."""
        if self._pending is not None:
            yield self._pending  # Nested: join the outer transaction.
            return
        self._pending = []
        try:
            yield self._pending
            steps = self._pending
        finally:
            pending, self._pending = self._pending, None
        if pending:
            self.apply(Transaction(steps))

    def cook(
        self,
        nodes: list[NodeRef] | NodeRef,
        time: Optional[float] = None,
        frame: Optional[int] = None,
        dt: Optional[float] = None,
    ) -> dict[str, CookResult]:
        """Cook nodes and return their outputs by node path. With ``time``
        (or ``frame``/``dt``) the host cooks on the session's own clock;
        without, at the scene's current clock. Needs ``graph.cook``."""
        refs = nodes if isinstance(nodes, list) else [nodes]
        params: dict[str, Any] = {
            "nodes": [n.id if isinstance(n, Node) else n for n in refs],
        }
        if time is not None:
            params["time"] = float(time)
        if frame is not None:
            params["frame"] = int(frame)
        if dt is not None:
            params["dt"] = float(dt)
        result = self._request("graph.cook", params)
        out: dict[str, CookResult] = {}
        for entry in result["results"]:
            outputs = {k: decode(v) for k, v in (entry.get("outputs") or {}).items()}
            path = entry.get("node") or entry.get("id")
            out[path] = CookResult(node=path, outputs=outputs, error=entry.get("error"))
        return out


F = TypeVar("F", bound=Callable[..., Any])


class Session:
    """A connection to the host, after the handshake.

    Attributes:
        capabilities: The granted capability tokens.
        room: The room's name.
        host: The host's name and version.
        limits: The host's limits (``request_timeout_ms`` and so on).
        graph: The graph API.
    """

    def __init__(self, reader: TextIO, writer: TextIO, client: str = CLIENT,
                 timeout: Optional[float] = None) -> None:
        """Connect over two text streams and say hello. Scripts normally
        call :func:`connect` instead."""
        self._conn = Connection(reader, writer)
        self._conn.handler = self._on_host_request
        self._timeout = timeout
        self._tick_hooks: list[Callable[[Tick], Any]] = []
        self._event_hooks: list[Callable[[Event], Any]] = []
        self._shutdown = False
        self._current: Any = None
        hello = self._conn.request(
            "hello", {"protocol": PROTOCOL_VERSION, "client": client}, timeout
        )
        if not isinstance(hello, dict) or hello.get("protocol") != PROTOCOL_VERSION:
            raise ProtocolError(f"unexpected hello answer: {hello!r}")
        self.capabilities: frozenset[str] = frozenset(hello.get("capabilities", []))
        self.room: str = (hello.get("room") or {}).get("name", "")
        self.host: str = hello.get("host", "")
        self.limits: dict[str, Any] = hello.get("limits", {})
        self.graph = Graph(self)

    def request(self, method: str, params: Any = None) -> Any:
        """Send a raw request (see the protocol document)."""
        return self._conn.request(method, params, self._timeout)

    def has(self, capability: str) -> bool:
        """Whether ``capability`` was granted."""
        return capability in self.capabilities

    def log(self, message: str, level: str = "info") -> None:
        """Send a log line to the host (``debug``, ``info``, ``warn``,
        ``error``)."""
        self._conn.notify("log", {"level": level, "message": str(message)})

    def debug(self, message: str) -> None:
        """Log at ``debug`` level."""
        self.log(message, "debug")

    def warn(self, message: str) -> None:
        """Log at ``warn`` level."""
        self.log(message, "warn")

    def error(self, message: str) -> None:
        """Log at ``error`` level."""
        self.log(message, "error")

    def on_tick(self, fn: F) -> F:
        """Decorator: call ``fn(tick)`` on every frame tick after
        :meth:`listen`."""
        self._tick_hooks.append(fn)
        return fn

    def on_event(self, fn: F) -> F:
        """Decorator: call ``fn(event)`` for every host event after
        :meth:`listen` (needs ``graph.read``)."""
        self._event_hooks.append(fn)
        return fn

    def cancelled(self) -> bool:
        """Inside a hook: whether the host cancelled the current tick or
        event (finish quickly; the host kills scripts that do not)."""
        return self._current is not None and self._conn.is_cancelled(self._current)

    def listen(self, topics: Optional[list[str]] = None) -> None:
        """End the main body and serve ticks and events until the host
        shuts the session down. ``topics`` defaults to those with hooks
        (``tick`` and/or ``event``)."""
        if topics is None:
            topics = []
            if self._tick_hooks:
                topics.append("tick")
            if self._event_hooks:
                topics.append("event")
        self.request("listen", {"topics": topics})
        try:
            self._conn.serve(lambda: self._shutdown)
        except ProtocolError:
            pass  # The host closed the connection: we are done.

    def _on_host_request(self, method: str, params: Any, request_id: Any) -> Any:
        params = params or {}
        self._current = request_id
        try:
            if method == "tick":
                tick = Tick(
                    frame=int(params.get("frame", 0)),
                    time=float(params.get("time", 0.0)),
                    dt=float(params.get("dt", 0.0)),
                    fired=int(params.get("fired", 0)),
                )
                for hook in self._tick_hooks:
                    hook(tick)
                return None
            if method == "event":
                event = Event(
                    kind=str(params.get("kind", "")),
                    node=params.get("node"),
                    port=params.get("port"),
                    payload=decode(params.get("payload")),
                    data=dict(params),
                )
                for hook in self._event_hooks:
                    hook(event)
                return None
            if method == "shutdown":
                self._shutdown = True
                return None
            raise RemoteError("unknown_method", f"unknown host method '{method}'")
        finally:
            self._current = None


_session: Optional[Session] = None


def _apply_memory_limit() -> None:
    limit = os.environ.get("LIVSPEIL_MEMORY_LIMIT")
    if not limit:
        return
    try:
        import resource

        value = int(limit)
        resource.setrlimit(resource.RLIMIT_AS, (value, value))
    except (ImportError, ValueError, OSError):
        pass  # Advisory: not every platform supports it.


def connect(client: str = CLIENT, timeout: Optional[float] = None) -> Session:
    """Connect to the host that started this script (once per process;
    later calls return the same session).

    Takes over the process's stdin and stdout for the protocol: afterwards
    ``print`` and anything else written to stdout goes to stderr, which the
    host shows as log lines, and stdin reads as empty.

    Raises:
        ProtocolError: the script was not started by a Livspeil host, or
            the handshake failed.
    """
    global _session
    if _session is not None:
        return _session
    if "LIVSPEIL_PROTOCOL" not in os.environ:
        raise ProtocolError(
            "not running under a Livspeil host; run the script with `livspeil run <room> <script>`"
        )
    _apply_memory_limit()
    sys.stdout.flush()
    proto_out = os.fdopen(os.dup(1), "w", encoding="utf-8", newline="\n")
    proto_in = os.fdopen(os.dup(0), "r", encoding="utf-8", newline="\n")
    os.dup2(2, 1)
    devnull = os.open(os.devnull, os.O_RDONLY)
    os.dup2(devnull, 0)
    os.close(devnull)
    _session = Session(proto_in, proto_out, client, timeout)
    return _session
