# Golden Battery CAN FD Scenarios

All scenarios contain exactly 200 CAN FD battery frames at 10 Hz (20 s).
Frame ID `0x100`, DLC code `0xA`, data length 16, BRS=0, ESI=0.

## Model-validation result

### `cold_nominal`
- start: min=-13.0, avg=-11.0, max=-9.5 °C, SoC=88.0%
- end: min=-2.0, avg=0.0, max=2.5 °C, SoC=83.5%
- detections: **none**

### `warm_nominal`
- start: min=27.5, avg=30.0, max=31.5 °C, SoC=45.0%
- end: min=35.5, avg=38.0, max=39.5 °C, SoC=52.0%
- detections: **none**

### `hot_nominal`
- start: min=56.5, avg=58.5, max=60.0 °C, SoC=78.0%
- end: min=56.5, avg=58.5, max=60.0 °C, SoC=68.5%
- detections:
  - `THERMAL_LIMIT/WARNING/temp_max`: 200 frames, first 0.0s, last 19.9s

### `overtemp_fault`
- start: min=54.0, avg=56.0, max=57.5 °C, SoC=84.0%
- end: min=70.5, avg=72.5, max=74.0 °C, SoC=74.5%
- detections:
  - `PHYSICAL_TEMP_RATE/WARNING/temp_min`: 33 frames, first 0.6s, last 19.8s
  - `PHYSICAL_TEMP_RATE/WARNING/temp_avg`: 33 frames, first 0.6s, last 19.8s
  - `PHYSICAL_TEMP_RATE/WARNING/temp_max`: 33 frames, first 0.6s, last 19.8s
  - `THERMAL_LIMIT/WARNING/temp_max`: 120 frames, first 3.0s, last 14.9s
  - `THERMAL_LIMIT/CRITICAL/temp_max`: 50 frames, first 15.0s, last 19.9s
  - `PHYSICAL_TEMP_ABSOLUTE_LIMIT/VIOLATION/temp_max`: 44 frames, first 15.6s, last 19.9s

### `hotspot_fault`
- start: min=42.5, avg=45.0, max=46.5 °C, SoC=72.0%
- end: min=44.5, avg=47.0, max=54.0 °C, SoC=67.5%
- detections:
  - `PHYSICAL_TEMP_HOTSPOT/WARNING/temp_max`: 10 frames, first 8.0s, last 8.9s
  - `PHYSICAL_TEMP_HOTSPOT/VIOLATION/temp_max`: 110 frames, first 9.0s, last 19.9s
  - `PHYSICAL_TEMP_SPREAD/WARNING/-`: 35 frames, first 12.5s, last 15.9s
  - `PHYSICAL_TEMP_SPREAD/VIOLATION/-`: 40 frames, first 16.0s, last 19.9s
