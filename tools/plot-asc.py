#!/usr/bin/env python3
# Copyright (c) 2026 Alwin Berger
#
# This program and the accompanying materials are made available under
# the terms of the Eclipse Public License 2.0 which accompanies this
# distribution, and is available at https://www.eclipse.org/legal/epl-2.0/
#
# AI Disclosure: This file was fully AI-generated.
#
# SPDX-License-Identifier: EPL-2.0 and CC0-1.0
# Assisted-by: deepseek-v4.1-flash
"""Plot the battery signals from an example CAN .asc file.

Frame 0x100 payload (8 bytes, little-endian):
CellTempAvg, CellTempMax, CellTempMin (scale 0.5, offset -40),
StateOfCharge (scale 0.5). Works for both the plain 8-byte demo file and
the fault-injection file with a 4-byte timestamp prefix.

The plot is written to a PNG (default: <asc-name>.png in the current
directory) and, if a GUI backend is available, also shown in a window.
Usage: plot-asc [asc-file] [output.png]
"""
import os
import sys
from pathlib import Path

import matplotlib

if "DISPLAY" not in os.environ and "WAYLAND_DISPLAY" not in os.environ:
    matplotlib.use("Agg")  # headless: no window, only save the PNG

import matplotlib.pyplot as plt

default = Path(__file__).resolve().parents[2] / "demo/can/battery_temp.asc"
asc = Path(sys.argv[1]) if len(sys.argv) > 1 else default
out = Path(sys.argv[2]) if len(sys.argv) > 2 else Path.cwd() / f"{asc.stem}.png"

t, avg, tmax, tmin, soc = [], [], [], [], []
for line in asc.read_text().splitlines():
    parts = line.split()
    if "d" not in parts or len(parts) < 10:  # skip header/non-data lines
        continue
    b = [int(x, 16) for x in parts[-8:]]  # last 8 bytes = CAN payload
    t.append(float(parts[0]))
    avg.append(int.from_bytes(bytes(b[0:2]), "little") * 0.5 - 40)
    tmax.append(int.from_bytes(bytes(b[2:4]), "little") * 0.5 - 40)
    tmin.append(int.from_bytes(bytes(b[4:6]), "little") * 0.5 - 40)
    soc.append(int.from_bytes(bytes(b[6:8]), "little") * 0.5)

fig, (ax0, ax1) = plt.subplots(2, 1, sharex=True)
ax0.plot(t, avg, label="CellTempAvg")
ax0.plot(t, tmax, label="CellTempMax")
ax0.plot(t, tmin, label="CellTempMin")
ax0.set_ylabel("Temperature (°C)")
ax0.legend()
ax1.plot(t, soc, color="tab:green")
ax1.set_ylabel("SoC (%)")
ax1.set_xlabel("time (s)")
fig.suptitle(asc.name)
fig.tight_layout()
fig.savefig(out, dpi=120)
print(f"saved: {out}")
plt.show()
