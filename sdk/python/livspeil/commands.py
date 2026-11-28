# SPDX-FileCopyrightText: 2026 Livspeil contributors
# SPDX-License-Identifier: MPL-2.0

"""Commands: the only way a script changes a document.

Each class mirrors a variant of the kernel's ``livspeil_cmd::Command`` and
encodes to its serde form (``{"set_input": {...}}``). Nodes may be given as
:class:`~livspeil.session.Node` objects, node ids, or paths; the session
resolves paths to ids before sending. Commands are applied with
:meth:`livspeil.session.Graph.apply`; every applied command is validated,
undoable and journaled with origin ``script``.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Any, Callable, Optional, Union

from .values import Value, encode

if TYPE_CHECKING:
    from .session import Node

NodeRef = Union["Node", str]
"""A node: a :class:`~livspeil.session.Node`, a node id, or a path."""

Resolver = Callable[[NodeRef], str]
"""Maps a node reference to its id."""


class Command:
    """Base class of commands."""

    def to_json(self, resolve: Resolver) -> dict[str, Any]:
        """The wire form, with node references resolved by ``resolve``."""
        raise NotImplementedError


@dataclass
class CreateNode(Command):
    """Add a node of ``type_id`` (qualified, e.g. ``core.math.add@1``)
    called ``name``, in the root network or the subnet ``parent``.

    ``id`` is filled in by :meth:`~livspeil.session.Graph.apply` when left
    ``None``, so it can be read after applying.
    """

    type_id: str
    name: str
    parent: Optional[NodeRef] = None
    id: Optional[str] = None

    def to_json(self, resolve: Resolver) -> dict[str, Any]:
        if self.id is None:
            raise ValueError("CreateNode needs an id (Graph.apply allocates one)")
        if self.parent is None:
            return {"create_node": {"id": self.id, "type_id": self.type_id, "name": self.name}}
        return {
            "create_node_in": {
                "id": self.id,
                "parent": resolve(self.parent),
                "type_id": self.type_id,
                "name": self.name,
            }
        }


@dataclass
class DeleteNode(Command):
    """Remove a node (a subnet with everything inside) and its links."""

    node: NodeRef

    def to_json(self, resolve: Resolver) -> dict[str, Any]:
        return {"delete_node": {"id": resolve(self.node)}}


@dataclass
class SetInput(Command):
    """Set an input's static value."""

    node: NodeRef
    port: str
    value: Value

    def to_json(self, resolve: Resolver) -> dict[str, Any]:
        return {
            "set_input": {"node": resolve(self.node), "port": self.port, "value": encode(self.value)}
        }


@dataclass
class ClearInput(Command):
    """Forget an input's value so the spec default applies."""

    node: NodeRef
    port: str

    def to_json(self, resolve: Resolver) -> dict[str, Any]:
        return {"clear_input": {"node": resolve(self.node), "port": self.port}}


@dataclass
class Link(Command):
    """Connect output ``from_port`` of ``from_node`` to an unlinked input."""

    from_node: NodeRef
    from_port: str
    to_node: NodeRef
    to_port: str

    def to_json(self, resolve: Resolver) -> dict[str, Any]:
        return {
            "link": {
                "from": resolve(self.from_node),
                "from_port": self.from_port,
                "to": resolve(self.to_node),
                "to_port": self.to_port,
            }
        }


@dataclass
class Unlink(Command):
    """Remove the link feeding an input (a no-op when unlinked)."""

    to_node: NodeRef
    to_port: str

    def to_json(self, resolve: Resolver) -> dict[str, Any]:
        return {"unlink": {"to": resolve(self.to_node), "to_port": self.to_port}}


@dataclass
class Rename(Command):
    """Rename a node (``ch()`` paths that name it are rewritten)."""

    node: NodeRef
    name: str

    def to_json(self, resolve: Resolver) -> dict[str, Any]:
        return {"rename": {"node": resolve(self.node), "name": self.name}}


@dataclass
class SetExpression(Command):
    """Drive an input with an expression such as ``ch("sun/offset") * 2``."""

    node: NodeRef
    port: str
    source: str

    def to_json(self, resolve: Resolver) -> dict[str, Any]:
        return {
            "set_expression": {"node": resolve(self.node), "port": self.port, "source": self.source}
        }


@dataclass
class ClearExpression(Command):
    """Remove an input's expression."""

    node: NodeRef
    port: str

    def to_json(self, resolve: Resolver) -> dict[str, Any]:
        return {"clear_expression": {"node": resolve(self.node), "port": self.port}}


@dataclass
class ClearAnimation(Command):
    """Remove an input's animation channel."""

    node: NodeRef
    port: str

    def to_json(self, resolve: Resolver) -> dict[str, Any]:
        return {"clear_animation": {"node": resolve(self.node), "port": self.port}}


@dataclass
class SetPosition(Command):
    """Place a node in the node editor, or unplace it with ``None``. Layout
    only: nothing is re-cooked."""

    node: NodeRef
    position: Optional[tuple[float, float]]

    def to_json(self, resolve: Resolver) -> dict[str, Any]:
        position = None
        if self.position is not None:
            x, y = self.position
            position = [float(x), float(y)]
        return {"set_position": {"node": resolve(self.node), "position": position}}


@dataclass
class Transaction(Command):
    """Apply ``steps`` in order, all or nothing, as one undo step."""

    steps: list[Command] = field(default_factory=list)

    def to_json(self, resolve: Resolver) -> dict[str, Any]:
        return {"transaction": [step.to_json(resolve) for step in self.steps]}


@dataclass
class RawCommand(Command):
    """A command already in wire form, for variants this SDK has no class
    for (``set_animation``, ``move_node``, ``promote``, ...)."""

    wire: dict[str, Any]

    def to_json(self, resolve: Resolver) -> dict[str, Any]:
        return self.wire


def walk(command: Command) -> list[Command]:
    """``command`` and, for transactions, every nested step."""
    out: list[Command] = [command]
    if isinstance(command, Transaction):
        for step in command.steps:
            out.extend(walk(step))
    return out
