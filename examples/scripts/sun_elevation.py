# SPDX-FileCopyrightText: 2026 Livspeil contributors
# SPDX-License-Identifier: CC0-1.0

"""Print the sun's elevation over a day by cooking the scene.

In sun-and-lamp the clock is sped up 8640x and the day starts at 06:00, so
hour h is at scene time ((h - 6) mod 24) / 2.4 seconds. Cooking never
changes the room.

    livspeil run examples/sun-and-lamp.room examples/scripts/sun_elevation.py \
        --grant graph.cook
"""

import livspeil

session = livspeil.connect()

for hour in range(0, 24, 2):
    time = ((hour - 6) % 24) / 2.4
    result = session.graph.cook(["time_of_day", "sun"], time=time)
    clock = result["/time_of_day"]["hour"]
    elevation = result["/sun"]["elevation"]
    bar = "#" * max(0, round(elevation / 3))
    session.log(f"{clock:05.2f}h  elevation {elevation:6.1f} deg  {bar}")
