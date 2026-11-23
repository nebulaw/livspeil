# SPDX-FileCopyrightText: 2026 Livspeil contributors
# SPDX-License-Identifier: MPL-2.0

"""The wire layer: one JSON object per line, requests, responses and
notifications in both directions."""

from __future__ import annotations

import json
import queue
import threading
import traceback
from typing import Any, Callable, Optional, TextIO

from .errors import ProtocolError, RemoteError, RequestTimeout, remote_error

HostHandler = Callable[[str, Any, int], Any]
"""Called for each host request as ``handler(method, params, id)``; its
return value is the result. Raising :class:`RemoteError` answers with that
error; any other exception answers ``handler_failed``."""

_EOF = object()


class Connection:
    """A bidirectional message channel over two text streams.

    A reader thread decodes incoming lines into a queue, so waits can time
    out and ``cancel`` notifications are seen while a handler runs.
    Requests are synchronous: :meth:`request` serves host requests that
    arrive while it waits (a script handling ``tick`` may issue requests of
    its own).
    """

    def __init__(self, reader: TextIO, writer: TextIO) -> None:
        self._writer = writer
        self._write_lock = threading.Lock()
        self._incoming: "queue.Queue[Any]" = queue.Queue()
        self._next_id = 0
        self._cancelled: set[Any] = set()
        self._cancel_lock = threading.Lock()
        self.handler: Optional[HostHandler] = None
        self.closed = False
        thread = threading.Thread(target=self._read, args=(reader,), daemon=True)
        thread.start()

    def _read(self, reader: TextIO) -> None:
        try:
            for line in reader:
                line = line.strip()
                if not line:
                    continue
                try:
                    message = json.loads(line)
                except ValueError:
                    continue
                if (
                    isinstance(message, dict)
                    and message.get("method") == "cancel"
                    and "id" not in message
                ):
                    params = message.get("params") or {}
                    with self._cancel_lock:
                        self._cancelled.add(params.get("id"))
                    continue
                self._incoming.put(message)
        except (OSError, ValueError):
            pass
        self._incoming.put(_EOF)

    def send(self, message: dict[str, Any]) -> None:
        """Write one message as a line."""
        line = json.dumps(message, separators=(",", ":"), allow_nan=False)
        with self._write_lock:
            self._writer.write(line + "\n")
            self._writer.flush()

    def notify(self, method: str, params: Any = None) -> None:
        """Send a notification (no answer)."""
        self.send({"method": method, "params": params if params is not None else {}})

    def is_cancelled(self, request_id: Any) -> bool:
        """Whether the host cancelled its request ``request_id``."""
        with self._cancel_lock:
            return request_id in self._cancelled

    def request(self, method: str, params: Any = None, timeout: Optional[float] = None) -> Any:
        """Send a request and wait for its result.

        Raises:
            RemoteError: the host answered with an error (or a subclass).
            RequestTimeout: no answer within ``timeout`` seconds (the
                request is cancelled).
            ProtocolError: the connection closed.
        """
        self._next_id += 1
        request_id = self._next_id
        self.send(
            {"id": request_id, "method": method, "params": params if params is not None else {}}
        )
        while True:
            message = self._next(timeout)
            if message is None:
                self.notify("cancel", {"id": request_id})
                raise RequestTimeout(f"no answer to '{method}' within {timeout} s")
            if "method" in message:
                self._dispatch(message)
                continue
            if message.get("id") != request_id:
                continue  # A stale answer.
            if "error" in message:
                raise remote_error(message["error"])
            return message.get("result")

    def serve(self, stop: Callable[[], bool]) -> None:
        """Serve host requests until ``stop()`` is true after one, or the
        connection closes."""
        while not stop():
            message = self._next(None)
            if message is None:
                return
            if "method" in message:
                self._dispatch(message)

    def _next(self, timeout: Optional[float]) -> Optional[dict[str, Any]]:
        if self.closed:
            raise ProtocolError("the connection is closed")
        try:
            item = self._incoming.get(timeout=timeout)
        except queue.Empty:
            return None
        if item is _EOF:
            self.closed = True
            raise ProtocolError("the host closed the connection")
        if not isinstance(item, dict):
            return self._next(timeout)
        return item

    def _dispatch(self, message: dict[str, Any]) -> None:
        method = message.get("method")
        if "id" not in message:
            return  # Unknown notifications are ignored.
        request_id = message["id"]
        try:
            if self.handler is None:
                raise RemoteError("unknown_method", f"no handler for '{method}'")
            result = self.handler(str(method), message.get("params"), request_id)
            if self.is_cancelled(request_id):
                raise RemoteError("cancelled", "the host cancelled the request")
            self.send({"id": request_id, "result": result})
        except RemoteError as err:
            error: dict[str, Any] = {"code": err.code, "message": err.message}
            if err.data is not None:
                error["data"] = err.data
            self.send({"id": request_id, "error": error})
        except Exception as err:  # noqa: BLE001 - reported to the host
            traceback.print_exc()
            self.send(
                {
                    "id": request_id,
                    "error": {
                        "code": "handler_failed",
                        "message": f"{type(err).__name__}: {err}",
                    },
                }
            )
        finally:
            with self._cancel_lock:
                self._cancelled.discard(request_id)
