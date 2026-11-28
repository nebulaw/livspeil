# SPDX-FileCopyrightText: 2026 Livspeil contributors
# SPDX-License-Identifier: MPL-2.0

"""Unit tests of the SDK against an in-process fake host (stdlib only).

Run from ``sdk/python``: ``python3 -m unittest discover -s tests``
"""

from __future__ import annotations

import contextlib
import io
import json
import os
import threading
import unittest
from typing import Any, Callable

import livspeil
from livspeil import (
    EVENT,
    Color,
    CreateNode,
    Link,
    RawValue,
    SetInput,
    Transaction,
    Vec3,
)
from livspeil.session import Session
from livspeil.values import decode, encode

LAMP = "0192f5a0-7c3e-7a10-8000-000000000009"
SUN = "0192f5a0-7c3e-7a10-8000-000000000004"
NEW = "0192f5a0-7c3e-7a10-8000-0000000000aa"

Handler = Callable[[str, Any], Any]


class FakeHost:
    """Speaks the host side over two pipes. ``handle(method, params)``
    returns a result, or raises :class:`livspeil.RemoteError`."""

    def __init__(self, handle: Handler, capabilities: list[str] | None = None) -> None:
        self.handle = handle
        self.capabilities = capabilities or ["graph.read", "graph.write", "graph.cook"]
        self.requests: list[dict[str, Any]] = []
        self.notifications: list[dict[str, Any]] = []
        to_script_r, to_script_w = os.pipe()
        to_host_r, to_host_w = os.pipe()
        self.script_in = os.fdopen(to_script_r, "r", encoding="utf-8")
        self.script_out = os.fdopen(to_host_w, "w", encoding="utf-8")
        self._in = os.fdopen(to_host_r, "r", encoding="utf-8")
        self._out = os.fdopen(to_script_w, "w", encoding="utf-8")
        self._lock = threading.Lock()
        self.responses: dict[int, dict[str, Any]] = {}
        self._got_response = threading.Condition(self._lock)
        self._thread = threading.Thread(target=self._serve, daemon=True)
        self._thread.start()

    def send(self, message: dict[str, Any]) -> None:
        with self._lock:
            try:
                self._out.write(json.dumps(message) + "\n")
                self._out.flush()
            except (OSError, ValueError):
                pass  # The test hung up.

    def call(self, request_id: int, method: str, params: Any = None) -> dict[str, Any]:
        """Send a host request and wait for the script's answer."""
        self.send({"id": request_id, "method": method, "params": params or {}})
        with self._got_response:
            while request_id not in self.responses:
                if not self._got_response.wait(timeout=5):
                    raise AssertionError(f"no answer to {method}")
            return self.responses[request_id]

    def hangup(self) -> None:
        """Close the host's sending side (the script sees end of file)."""
        with self._lock:
            self._out.close()

    def close(self) -> None:
        """Close every pipe end and stop the serving thread."""
        self.script_out.close()
        self._thread.join(timeout=5)
        with self._lock:
            if not self._out.closed:
                self._out.close()
        self._in.close()
        self.script_in.close()

    def _serve(self) -> None:
        for line in self._in:
            message = json.loads(line)
            if "method" not in message:
                with self._got_response:
                    self.responses[message["id"]] = message
                    self._got_response.notify_all()
                continue
            if "id" not in message:
                self.notifications.append(message)
                continue
            self.requests.append(message)
            method, params = message["method"], message.get("params")
            try:
                if method == "hello":
                    result: Any = {
                        "protocol": 1,
                        "host": "fake",
                        "capabilities": self.capabilities,
                        "room": {"name": "Test"},
                        "limits": {"request_timeout_ms": 5000},
                    }
                elif method == "listen":
                    result = {"topics": params.get("topics")}
                else:
                    result = self.handle(method, params)
                self.send({"id": message["id"], "result": result})
            except livspeil.RemoteError as err:
                error = {"code": err.code, "message": err.message}
                if err.data is not None:
                    error["data"] = err.data
                self.send({"id": message["id"], "error": error})


def lamp_info() -> dict[str, Any]:
    return {
        "id": LAMP,
        "name": "lamp",
        "path": "/lamp",
        "type": "core.math.remap@1",
        "parent": None,
        "subnet": False,
        "inputs": [
            {"name": "out_min", "type": "Float", "value": {"Float": 1.0},
             "default": {"Float": 0.0}, "link": None, "expression": None, "animated": False},
            {"name": "in_min", "type": "Float", "value": None,
             "default": {"Float": 0.0}, "link": None, "expression": None, "animated": False},
            {"name": "x", "type": "Float", "value": None, "default": {"Float": 0.0},
             "link": {"node": "/daylight", "id": SUN, "port": "out"},
             "expression": None, "animated": False},
        ],
        "outputs": [{"name": "out", "type": "Float", "links": []}],
    }


def default_handler(method: str, params: Any) -> Any:
    if method == "graph.nodes":
        return {"nodes": [
            {"id": LAMP, "name": "lamp", "path": "/lamp", "type": "core.math.remap@1",
             "parent": None, "subnet": False},
            {"id": SUN, "name": "sun", "path": "/sun", "type": "core.sky.sun_direction@1",
             "parent": None, "subnet": False},
        ]}
    if method == "graph.node":
        if params["node"] in ("lamp", "/lamp", LAMP):
            return lamp_info()
        raise livspeil.NotFound("not_found", f"no node '{params['node']}'")
    if method == "graph.apply":
        return {"changes": [{"kind": "input_changed"}], "undo_len": 1}
    if method == "graph.new_id":
        return {"ids": [NEW] * params.get("count", 1)}
    if method == "graph.cook":
        return {
            "clock": {"frame": 0, "time": params.get("time", 0.0), "dt": 0.0},
            "results": [
                {"node": "/sun", "id": SUN,
                 "outputs": {"elevation": {"Float": 12.5}, "direction": {"Vec3": [0, 1, 0]}}},
                {"node": "/lamp", "id": LAMP, "error": "upstream failed"},
            ],
        }
    raise livspeil.RemoteError("unknown_method", method)


class SessionTest(unittest.TestCase):
    def connect(self, handler: Handler = default_handler, **kw: Any) -> tuple[Session, FakeHost]:
        host = FakeHost(handler, **kw)
        session = Session(host.script_in, host.script_out, client="unit", timeout=5)
        self.addCleanup(host.close)
        return session, host

    def test_handshake(self) -> None:
        session, host = self.connect(capabilities=["graph.read"])
        self.assertEqual(session.capabilities, frozenset({"graph.read"}))
        self.assertEqual(session.room, "Test")
        self.assertTrue(session.has("graph.read"))
        self.assertFalse(session.has("graph.write"))
        hello = host.requests[0]
        self.assertEqual(hello["method"], "hello")
        self.assertEqual(hello["params"], {"protocol": 1, "client": "unit"})

    def test_nodes_and_inputs(self) -> None:
        session, _ = self.connect()
        nodes = session.graph.nodes()
        self.assertEqual([n.path for n in nodes], ["/lamp", "/sun"])
        self.assertEqual([n.name for n in session.graph.nodes(type="core.sky.sun_direction")],
                         ["sun"])
        lamp = nodes[0]
        self.assertEqual(lamp.input("out_min"), 1.0)
        self.assertEqual(lamp.input("in_min"), 0.0)  # default
        self.assertEqual(lamp.inputs["x"].link, "/daylight.out")
        self.assertEqual(lamp.outputs["out"].type, "Float")
        with self.assertRaises(KeyError):
            lamp.input("nope")
        with self.assertRaises(livspeil.NotFound):
            session.graph.node("ghost")

    def test_apply_resolves_paths_and_sends_wire_form(self) -> None:
        session, host = self.connect()
        result = session.graph.apply(SetInput("lamp", "out_min", 0.5), group=3)
        assert result is not None
        self.assertEqual(result.undo_len, 1)
        apply = [r for r in host.requests if r["method"] == "graph.apply"][-1]
        self.assertEqual(apply["params"], {
            "command": {"set_input": {"node": LAMP, "port": "out_min", "value": {"Float": 0.5}}},
            "group": 3,
        })

    def test_transaction_batches_and_allocates_ids(self) -> None:
        session, host = self.connect()
        with session.graph.transaction() as pending:
            node = CreateNode("core.math.mul@1", "half")
            self.assertIsNone(session.graph.apply(node))
            session.graph.apply(Link(SUN, "elevation", node.id or "", "a"))
            self.assertEqual(len(pending), 2)
        self.assertEqual(node.id, NEW)
        applies = [r for r in host.requests if r["method"] == "graph.apply"]
        self.assertEqual(len(applies), 1)
        steps = applies[0]["params"]["command"]["transaction"]
        self.assertEqual(steps[0], {"create_node": {"id": NEW, "type_id": "core.math.mul@1",
                                                    "name": "half"}})
        self.assertEqual(steps[1]["link"]["from"], SUN)

    def test_transaction_discarded_on_exception(self) -> None:
        session, host = self.connect()
        with self.assertRaises(ZeroDivisionError):
            with session.graph.transaction():
                session.graph.apply(SetInput(LAMP, "out_min", 0.5))
                _ = 1 / 0
        self.assertFalse([r for r in host.requests if r["method"] == "graph.apply"])

    def test_errors_map_to_exceptions(self) -> None:
        def handler(method: str, params: Any) -> Any:
            if method == "graph.apply":
                raise livspeil.RemoteError(
                    "capability_denied", "no", {"capability": "graph.write"})
            if method == "graph.cook":
                raise livspeil.RemoteError("command_failed", "bad")
            return default_handler(method, params)

        session, _ = self.connect(handler)
        with self.assertRaises(livspeil.CapabilityDenied) as denied:
            session.graph.apply(SetInput(LAMP, "out_min", 0.5))
        self.assertEqual(denied.exception.data, {"capability": "graph.write"})
        with self.assertRaises(livspeil.CommandFailed):
            session.graph.cook(["sun"])

    def test_cook_decodes_values(self) -> None:
        session, host = self.connect()
        results = session.graph.cook(["sun", "lamp"], time=2.0)
        self.assertEqual(results["/sun"]["elevation"], 12.5)
        self.assertEqual(results["/sun"]["direction"], Vec3(0.0, 1.0, 0.0))
        self.assertEqual(results["/lamp"].error, "upstream failed")
        cook = [r for r in host.requests if r["method"] == "graph.cook"][-1]
        self.assertEqual(cook["params"], {"nodes": ["sun", "lamp"], "time": 2.0})

    def test_log_is_a_notification(self) -> None:
        session, host = self.connect()
        session.warn("careful")
        session.graph.nodes()  # Round trip so the notification has arrived.
        self.assertEqual(host.notifications[-1],
                         {"method": "log", "params": {"level": "warn", "message": "careful"}})

    def test_hooks_ticks_events_and_shutdown(self) -> None:
        session, host = self.connect()
        seen: list[Any] = []

        @session.on_tick
        def tick(t: livspeil.Tick) -> None:
            seen.append(("tick", t.frame, t.time))
            if t.frame == 2:
                raise ValueError("bad frame")

        @session.on_event
        def event(e: livspeil.Event) -> None:
            seen.append(("event", e.kind, e.node, e.payload))

        thread = threading.Thread(target=session.listen)
        thread.start()
        quiet = contextlib.redirect_stderr(io.StringIO())
        quiet.__enter__()
        self.addCleanup(quiet.__exit__, None, None, None)
        self.assertEqual(host.call(100, "tick", {"frame": 1, "time": 0.5, "dt": 0.5}),
                         {"id": 100, "result": None})
        answer = host.call(101, "tick", {"frame": 2, "time": 1.0, "dt": 0.5})
        self.assertEqual(answer["error"]["code"], "handler_failed")
        self.assertIn("bad frame", answer["error"]["message"])
        host.call(102, "event", {"kind": "scene", "node": "/blink", "port": "fired",
                                 "payload": {"Int": 3}})
        self.assertEqual(host.call(103, "bogus")["error"]["code"], "unknown_method")
        host.call(104, "shutdown", {"reason": "host"})
        thread.join(timeout=5)
        self.assertFalse(thread.is_alive())
        self.assertEqual(seen, [("tick", 1, 0.5), ("tick", 2, 1.0),
                                ("event", "scene", "/blink", 3)])
        listen = [r for r in host.requests if r["method"] == "listen"][0]
        self.assertEqual(listen["params"], {"topics": ["tick", "event"]})

    def test_request_timeout_sends_cancel(self) -> None:
        gate = threading.Event()

        def slow(method: str, params: Any) -> Any:
            gate.wait(5)
            return default_handler(method, params)

        session, host = self.connect(slow)
        session._timeout = 0.2
        with self.assertRaises(livspeil.RequestTimeout):
            session.graph.nodes()
        gate.set()
        session._timeout = 5
        session.graph.nodes()  # The stale answer is skipped.
        self.assertEqual(host.notifications[-1]["method"], "cancel")

    def test_closed_connection(self) -> None:
        session, host = self.connect()
        host.hangup()
        with self.assertRaises(livspeil.ProtocolError):
            session.graph.nodes()


class ValuesTest(unittest.TestCase):
    def test_round_trip(self) -> None:
        for value in [1.5, 3, True, False, "hi", Vec3(1.0, 2.0, 3.0),
                      Color(0.5, 0.1, 60.0, 1.0), EVENT]:
            self.assertEqual(decode(encode(value)), value)
        self.assertEqual(encode(True), {"Bool": True})
        self.assertEqual(encode(2), {"Int": 2})
        self.assertEqual(encode(EVENT), "Event")
        with self.assertRaises(TypeError):
            encode([1, 2])  # type: ignore[arg-type]

    def test_unknown_values_stay_raw(self) -> None:
        table = {"Table": {"columns": []}}
        raw = decode(table)
        self.assertEqual(raw, RawValue(table))
        self.assertEqual(encode(raw), table)  # type: ignore[arg-type]
        self.assertIsNone(decode(None))
        self.assertIsNone(decode({"Float": None}))

    def test_color_scaled(self) -> None:
        self.assertEqual(Color(0.5, 0.1, 60.0).scaled(0.5), Color(0.25, 0.1, 60.0, 1.0))


class CommandsTest(unittest.TestCase):
    def test_wire_forms(self) -> None:
        ident = lambda ref: str(ref)  # noqa: E731
        self.assertEqual(
            Transaction([SetInput("a", "x", 1.0), CreateNode("t@1", "n", parent="p", id="i")])
            .to_json(ident),
            {"transaction": [
                {"set_input": {"node": "a", "port": "x", "value": {"Float": 1.0}}},
                {"create_node_in": {"id": "i", "parent": "p", "type_id": "t@1", "name": "n"}},
            ]},
        )
        with self.assertRaises(ValueError):
            CreateNode("t@1", "n").to_json(ident)
        self.assertEqual(
            livspeil.SetPosition("a", (1, -2.5)).to_json(ident),
            {"set_position": {"node": "a", "position": [1.0, -2.5]}},
        )
        # Unplacing sends the field: the host refuses a command without it.
        self.assertEqual(
            livspeil.SetPosition("a", None).to_json(ident),
            {"set_position": {"node": "a", "position": None}},
        )

    def test_connect_outside_a_host(self) -> None:
        saved = os.environ.pop("LIVSPEIL_PROTOCOL", None)
        try:
            with self.assertRaises(livspeil.ProtocolError):
                livspeil.connect()
        finally:
            if saved is not None:
                os.environ["LIVSPEIL_PROTOCOL"] = saved


if __name__ == "__main__":
    unittest.main()
