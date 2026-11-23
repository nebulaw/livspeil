# SPDX-FileCopyrightText: 2026 Livspeil contributors
# SPDX-License-Identifier: MPL-2.0

"""Exceptions raised by the SDK."""

from __future__ import annotations

from typing import Any


class LivspeilError(Exception):
    """Base class of every SDK error."""


class ProtocolError(LivspeilError):
    """The host sent something the SDK cannot understand, or the
    connection closed unexpectedly."""


class RequestTimeout(LivspeilError):
    """The host did not answer within the client-side timeout."""


class RemoteError(LivspeilError):
    """The host answered a request with an error.

    Attributes:
        code: Stable error code, for example ``"not_found"``.
        message: Human-readable description.
        data: Optional structured details.
    """

    def __init__(self, code: str, message: str, data: Any = None) -> None:
        super().__init__(f"{code}: {message}")
        self.code = code
        self.message = message
        self.data = data


class CapabilityDenied(RemoteError):
    """The session lacks the capability the request needs
    (``data["capability"]`` names it)."""


class NotFound(RemoteError):
    """A node path or id does not name a node."""


class CommandFailed(RemoteError):
    """The command bus rejected a command; the graph is unchanged."""


_BY_CODE: dict[str, type[RemoteError]] = {
    "capability_denied": CapabilityDenied,
    "not_found": NotFound,
    "command_failed": CommandFailed,
}


def remote_error(error: Any) -> RemoteError:
    """Build the matching exception from a response's ``error`` object."""
    if not isinstance(error, dict):
        return RemoteError("internal", f"malformed error: {error!r}")
    code = str(error.get("code", "internal"))
    cls = _BY_CODE.get(code, RemoteError)
    return cls(code, str(error.get("message", "")), error.get("data"))
