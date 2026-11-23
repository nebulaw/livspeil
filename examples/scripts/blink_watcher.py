# SPDX-FileCopyrightText: 2026 Livspeil contributors
# SPDX-License-Identifier: CC0-1.0

"""React to the scene: count the blink timer's events and, after the third
blink, make the lamp's bright state softer (an undoable edit made from an
event hook).

    livspeil run examples/blink.room examples/scripts/blink_watcher.py \
        --grant graph.read,graph.write --ticks 40 --fps 20
"""

import livspeil
from livspeil import SetInput

session = livspeil.connect()
blinks = 0


@session.on_event
def on_event(event: livspeil.Event) -> None:
    global blinks
    if event.kind == "scene" and event.node == "/blink" and event.port == "fired":
        blinks += 1
        session.log(f"blink {blinks}")
        if blinks == 3:
            session.graph.apply(SetInput("brightness", "a", 0.6))
            session.log("softened the lamp")


@session.on_tick
def on_tick(tick: livspeil.Tick) -> None:
    if tick.frame % 20 == 0:
        session.debug(f"frame {tick.frame} at {tick.time:.2f} s")


session.listen()
session.log(f"saw {blinks} blinks")
