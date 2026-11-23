# SPDX-FileCopyrightText: 2026 Livspeil contributors
# SPDX-License-Identifier: MPL-2.0

"""Node values and their wire form.

On the wire a value is serde's externally tagged form of the kernel's
``Value``: ``{"Float": 1.5}``, ``{"Int": 3}``, ``{"Bool": true}``,
``{"String": "hi"}``, ``{"Vec3": [0, 1, 0]}``, ``{"Color": [l, c, h, a]}``
(OKLCH plus alpha), ``"Event"``, and objects for ``Table`` and ``Curve``.

In Python: ``float``, ``int``, ``bool``, ``str``, :class:`Vec2`,
:class:`Vec3`, :class:`Vec4`, :class:`Color`, :data:`EVENT`, and
:class:`RawValue` for tables and curves (kept in wire form).
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Union


@dataclass(frozen=True)
class Vec2:
    """A 2-component vector."""

    x: float
    y: float


@dataclass(frozen=True)
class Vec3:
    """A 3-component vector."""

    x: float
    y: float
    z: float


@dataclass(frozen=True)
class Vec4:
    """A 4-component vector."""

    x: float
    y: float
    z: float
    w: float


@dataclass(frozen=True)
class Color:
    """An OKLCH color with alpha: lightness, chroma, hue in degrees."""

    l: float  # noqa: E741 - the OKLCH name
    c: float
    h: float
    a: float = 1.0

    def scaled(self, factor: float) -> "Color":
        """The same color with its lightness multiplied by ``factor``."""
        return Color(self.l * factor, self.c, self.h, self.a)


@dataclass(frozen=True)
class RawValue:
    """A value kept in wire form (``Table``, ``Curve``, or a type this SDK
    does not know yet)."""

    wire: Any


class _Event:
    """The value of an ``Event`` port in value cooking (carries nothing)."""

    _instance: "_Event | None" = None

    def __new__(cls) -> "_Event":
        if cls._instance is None:
            cls._instance = super().__new__(cls)
        return cls._instance

    def __repr__(self) -> str:
        return "EVENT"


EVENT = _Event()
"""The single value of ``Event`` ports."""

Value = Union[float, int, bool, str, Vec2, Vec3, Vec4, Color, RawValue, _Event]
"""Any node value."""


def encode(value: Value) -> Any:
    """Turn a Python value into its wire form.

    Raises:
        TypeError: for a value with no wire form.
    """
    # bool before int: bool is a subclass of int.
    if isinstance(value, bool):
        return {"Bool": value}
    if isinstance(value, int):
        return {"Int": value}
    if isinstance(value, float):
        return {"Float": value}
    if isinstance(value, str):
        return {"String": value}
    if isinstance(value, Vec2):
        return {"Vec2": [float(value.x), float(value.y)]}
    if isinstance(value, Vec3):
        return {"Vec3": [float(value.x), float(value.y), float(value.z)]}
    if isinstance(value, Vec4):
        return {"Vec4": [float(value.x), float(value.y), float(value.z), float(value.w)]}
    if isinstance(value, Color):
        return {"Color": [float(value.l), float(value.c), float(value.h), float(value.a)]}
    if isinstance(value, RawValue):
        return value.wire
    if value is EVENT:
        return "Event"
    raise TypeError(f"cannot send {type(value).__name__} as a node value")


_VECTORS = {"Vec2": Vec2, "Vec3": Vec3, "Vec4": Vec4, "Color": Color}


def decode(wire: Any) -> Value | None:
    """Turn a wire value into Python. ``None`` stays ``None`` (an unset
    value, or a non-finite float the host could not send)."""
    if wire is None:
        return None
    if wire == "Event":
        return EVENT
    if isinstance(wire, dict) and len(wire) == 1:
        ((tag, inner),) = wire.items()
        if tag == "Float":
            return None if inner is None else float(inner)
        if tag == "Int":
            return int(inner)
        if tag == "Bool":
            return bool(inner)
        if tag == "String":
            return str(inner)
        cls = _VECTORS.get(tag)
        if cls is not None and isinstance(inner, list):
            return cls(*(float(x) for x in inner))
    return RawValue(wire)
