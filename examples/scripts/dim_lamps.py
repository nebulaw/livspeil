# SPDX-FileCopyrightText: 2026 Livspeil contributors
# SPDX-License-Identifier: CC0-1.0

"""Dim every lamp by 20%.

A lamp is any node whose name starts with "lamp": its Color inputs get 20%
less lightness and, for intensity remaps, the output range is scaled by
0.8. Every change is one transaction, so one undo step.

    livspeil run examples/sun-and-lamp.room examples/scripts/dim_lamps.py \
        --grant graph.read,graph.write
"""

import livspeil
from livspeil import Color, SetInput

FACTOR = 0.8

session = livspeil.connect()
graph = session.graph

lamps = [node for node in graph.nodes() if node.name.startswith("lamp")]
session.log(f"dimming {len(lamps)} lamp nodes by {round((1 - FACTOR) * 100)}%")

with graph.transaction():
    for lamp in lamps:
        for port in lamp.inputs.values():
            if port.link or port.expression or port.animated:
                continue  # Driven by something else: leave it alone.
            value = port.effective
            if isinstance(value, Color):
                dimmed = Color(round(value.l * FACTOR, 6), value.c, value.h, value.a)
                graph.apply(SetInput(lamp, port.name, dimmed))
            elif lamp.type.startswith("core.math.remap@") and port.name in ("out_min", "out_max"):
                graph.apply(SetInput(lamp, port.name, round(float(value) * FACTOR, 6)))
