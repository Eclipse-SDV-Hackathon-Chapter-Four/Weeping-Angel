# Battery Guardian: Physical Consistency Model

## 1. Scope

The Guardian observes

$$
x_k=(T_{\min,k},T_{\mathrm{avg},k},T_{\max,k},SoC_k)
$$

from the battery telemetry stream. The nominal VSS/DBC cycle is

$$
\Delta t_{\mathrm{nom}}=100\,\mathrm{ms},
$$

with temperature and SoC quantization of $0.5\,^\circ\mathrm C$ and $0.5$ percentage points.

The Guardian is deliberately **not** a detailed electro-thermal battery model. It checks a compact set of closed-form consistency constraints. Numeric values below are demonstrator parameters, not qualification limits for a particular battery chemistry.

For temporal checks, the model uses the **actual elapsed receive time**

$$
\Delta t_k=t^{recv}_k-t^{recv}_{k-1},
$$

not the nominal 100-ms period.

---

## 2. Configuration

### 2.1 Timing and warning policy

| Parameter | Value | Meaning |
|---|---:|---|
| nominal sample period | 100 ms | expected VSS/DBC cycle |
| evaluation period | 100 ms | Guardian task period |
| stale timeout $\tau_{\mathrm{stale}}$ | 500 ms | maximum age of the last valid sample |
| utilization warning threshold $u_{\mathrm{warning}}$ | 0.8 | warning fraction for continuous limits |

For a continuous upper bound,

$$
u=\frac{\mathrm{observed}}{\mathrm{limit}},
\qquad
r=\mathrm{observed}-\mathrm{limit}.
$$

Classification:

$$
u<0.8:\ \text{no detection},\qquad
0.8\le u\le1:\ \texttt{WARNING},\qquad
u>1:\ \texttt{VIOLATION}.
$$

A warning may therefore have $r\le0$.

### 2.2 Temperature envelope

| Parameter | Symbol | Value |
|---|---:|---:|
| absolute minimum | $T_{\mathrm{abs,min}}$ | $-30\,^\circ\mathrm C$ |
| absolute maximum | $T_{\mathrm{abs,max}}$ | $70\,^\circ\mathrm C$ |
| warning margin | $M_{\mathrm{warning}}$ | $10\,^\circ\mathrm C$ |
| reference temperature | $T_{\mathrm{ref}}$ | $20\,^\circ\mathrm C$ |
| hot-state temperature | $T_{\mathrm{hot}}$ | $70\,^\circ\mathrm C$ |

The operating thresholds are derived:

$$
T_{\mathrm{critical}}=T_{\mathrm{abs,max}},
\qquad
T_{\mathrm{warning}}=T_{\mathrm{abs,max}}-M_{\mathrm{warning}}.
$$

Hence:

- $T_\max<60^\circ\mathrm C$: no thermal-limit detection
- $60\le T_\max<70^\circ\mathrm C$: `THERMAL_LIMIT / WARNING`
- $T_\max\ge70^\circ\mathrm C$: `THERMAL_LIMIT / CRITICAL`

At exactly $70^\circ\mathrm C$, the state is critical but still inside the absolute envelope. `PHYSICAL_TEMP_ABSOLUTE_LIMIT` starts only above $70^\circ\mathrm C$.

Configuration invariant:

$$
T_{\mathrm{abs,min}}
\le T_{\mathrm{ref}}
< T_{\mathrm{hot}}
\le T_{\mathrm{abs,max}}.
$$

### 2.3 Temperature-dependent limits

Normalize the thermal state as

$$
\theta(T)=
\operatorname{clip}
\left(
\frac{T-T_{\mathrm{ref}}}
     {T_{\mathrm{hot}}-T_{\mathrm{ref}}},
0,1
\right).
$$

With the current parameters,

$$
S(T)=12-5\theta(T)
$$

is the maximum max-min spread and

$$
H(T)=5-3\theta(T)
$$

the maximum max-average hotspot deviation.

![Temperature-dependent spatial limits](guardian_spatial_limits_revised.png)

Dynamic parameters:

| Parameter | Value |
|---|---:|
| heating limit, cold/reference state | $8\,^\circ\mathrm C/s$ |
| heating limit, hot state | $5\,^\circ\mathrm C/s$ |
| cooling magnitude limit | $6\,^\circ\mathrm C/s$ |
| optional SoC coupling gain | $0.25\,^\circ\mathrm C/\mathrm{pp}$ |
| optional SoC-rate cap | $2\,\mathrm{pp/s}$ |

The SoC coupling is **disabled by default**. Gain and cap remain configured so the feature can be enabled without changing the model definition.

### 2.4 SoC

| Parameter | Value |
|---|---:|
| minimum | 0 % |
| maximum | 100 % |
| maximum absolute rate | 5 pp/s |

Using a rate rather than a fixed per-received-sample step avoids false detections after delay or packet loss.

### 2.5 Stuck detection

| Parameter | Value |
|---|---:|
| window length $N$ | 10 samples |
| flatness tolerance $\epsilon$ | $0.25^\circ\mathrm C$ |
| temperature excitation $E_T$ | $1.0^\circ\mathrm C$ |
| SoC excitation $E_{SoC}$ | 1.0 pp |

Stuck detection applies only to `temp_min`, `temp_avg`, and `temp_max`. SoC is an excitation source, not a stuck target.

---

## 3. Detection semantics

A Guardian observation is

```text
DetectionClass × DetectionLevel
```

Canonical classes:

```text
STREAM_STALE
THERMAL_LIMIT
PHYSICAL_TEMP_ABSOLUTE_LIMIT
PHYSICAL_TEMP_ORDERING
PHYSICAL_TEMP_SPREAD
PHYSICAL_TEMP_HOTSPOT
PHYSICAL_TEMP_RATE
PHYSICAL_SOC_RANGE
PHYSICAL_SOC_RATE
SIGNAL_STUCK
```

`WARNING` is used for `THERMAL_LIMIT`, spread, hotspot, and temperature rate. `CRITICAL` is used only for `THERMAL_LIMIT`. All remaining checks are binary and report `VIOLATION`.

---

## 4. Static and spatial temperature checks

### Absolute envelope

Check independently:

$$
T_\min\ge T_{\mathrm{abs,min}},
\qquad
T_\max\le T_{\mathrm{abs,max}}.
$$

A strict exceedance raises `PHYSICAL_TEMP_ABSOLUTE_LIMIT / VIOLATION`.

### Ordering

Require

$$
T_\min\le T_{\mathrm{avg}}\le T_\max.
$$

Violation raises `PHYSICAL_TEMP_ORDERING / VIOLATION`.

### Spread

$$
S_{\mathrm{obs}}=T_\max-T_\min,
\qquad
S_{\mathrm{lim}}=S(T_{\mathrm{avg}}).
$$

$$
u_S=\frac{S_{\mathrm{obs}}}{S_{\mathrm{lim}}},
\qquad
r_S=S_{\mathrm{obs}}-S_{\mathrm{lim}}.
$$

Class: `PHYSICAL_TEMP_SPREAD`.

### Hotspot

$$
H_{\mathrm{obs}}=T_\max-T_{\mathrm{avg}},
\qquad
H_{\mathrm{lim}}=H(T_{\mathrm{avg}}).
$$

$$
u_H=\frac{H_{\mathrm{obs}}}{H_{\mathrm{lim}}},
\qquad
r_H=H_{\mathrm{obs}}-H_{\mathrm{lim}}.
$$

Class: `PHYSICAL_TEMP_HOTSPOT`.

Spread and hotspot are separate by design: spread measures pack-wide non-uniformity; hotspot measures divergence of the hottest cell from the thermal bulk.

---

## 5. Dynamic temperature consistency

For each $i\in\{\min,\mathrm{avg},\max\}$,

$$
\dot T_{i,k}
=
\frac{T_{i,k}-T_{i,k-1}}{\Delta t_k}.
$$

Use the **previous** average temperature for the state-dependent heating limit:

$$
R_{\uparrow,\mathrm{base}}
=
8-3\theta(T_{\mathrm{avg},k-1})
\quad [^\circ\mathrm C/s].
$$

If SoC coupling is disabled,

$$
R_\uparrow=R_{\uparrow,\mathrm{base}}.
$$

If enabled,

$$
\dot{SoC}_k=
\frac{SoC_k-SoC_{k-1}}{\Delta t_k},
\qquad
Q_k=\min(|\dot{SoC}_k|,2),
$$

$$
R_\uparrow
=
R_{\uparrow,\mathrm{base}}
+0.25Q_k.
$$

For each temperature channel use

$$
(\mathrm{observed},\mathrm{limit})=
\begin{cases}
(\dot T_i,R_\uparrow), & \dot T_i\ge0,\\
(-\dot T_i,6), & \dot T_i<0.
\end{cases}
$$

and classify $u_R=\mathrm{observed}/\mathrm{limit}$ with the generic 0.8/1.0 policy.

Class: `PHYSICAL_TEMP_RATE`.

For the nominal 100-ms cycle the base positive step limit is 0.8 °C at the reference state and 0.5 °C at the hot state. This is illustrative only; runtime evaluation uses the actual $\Delta t_k$.

![Nominal 100-ms base heating step](guardian_heating_step_limit_revised.png)

The optional coupling is:

![Optional SoC-to-heating coupling](guardian_soc_rate_coupling_revised.png)

---

## 6. SoC consistency

### Range

Require

$$
0\le SoC_k\le100.
$$

Violation raises `PHYSICAL_SOC_RANGE / VIOLATION`.

### Rate

For consecutive **received** samples,

$$
\dot{SoC}_k=
\frac{SoC_k-SoC_{k-1}}{\Delta t_k}.
$$

Require

$$
|\dot{SoC}_k|\le5\,\mathrm{pp/s},
$$

equivalently

$$
|SoC_k-SoC_{k-1}|
\le5\,\Delta t_k.
$$

Violation raises `PHYSICAL_SOC_RATE / VIOLATION`. The first sample has no temporal checks.

---

## 7. Stuck detection

For each temperature channel over the last $N$ samples,

$$
A_i=\max(T_i)-\min(T_i),
$$

and for SoC over the same window,

$$
A_{SoC}=\max(SoC)-\min(SoC).
$$

A temperature signal is stuck iff

$$
A_i\le\epsilon
$$

and there is independent excitation:

$$
\max_{j\ne i}A_j\ge E_T
\quad\lor\quad
A_{SoC}\ge E_{SoC}.
$$

A positive result raises `SIGNAL_STUCK / VIOLATION`.

---

## 8. Stream freshness and evaluation

Let $t_{\mathrm{last}}$ be the receive time of the last valid sample:

$$
a(t)=t-t_{\mathrm{last}}.
$$

If

$$
a(t)>0.5\,s,
$$

raise `STREAM_STALE / VIOLATION`.

Freshness is checked periodically even without new input. Each received sample is physically evaluated at most once; the previous evaluated sample is retained for temporal checks.

The Guardian observes only the symptom:

```text
transport.delay  --\
transport.drop   ---+--> STREAM_STALE
source.dropout   --/
```

It does not infer the injected root cause.

---

## 9. Evaluation order

For each new valid sample:

1. classify `THERMAL_LIMIT`;
2. check absolute bounds and ordering;
3. evaluate spread and hotspot;
4. if a previous sample exists, evaluate temperature and SoC rates using actual $\Delta t_k$;
5. update and evaluate the stuck window;
6. emit detection state transitions and project configured class/level pairs to DFM.

Malformed messages do not update model state or freshness.

---

## 10. Evidence and injection semantics

For continuous checks, evidence should expose at least:

```yaml
detection_class: PHYSICAL_TEMP_SPREAD
detection_level: VIOLATION
observed: 13.0
limit: 9.3
residual: 3.7
utilization: 1.398
```

Temporal evidence should additionally include the affected signal and actual $\Delta t_k$.

Injected faults are causes; Guardian detections are observations:

| Injection | Typical observation |
|---|---|
| temperature `signal.spike` | `PHYSICAL_TEMP_RATE` |
| SoC `signal.spike` | `PHYSICAL_SOC_RATE` |
| temperature `signal.drift` | `PHYSICAL_TEMP_SPREAD` and/or `PHYSICAL_TEMP_HOTSPOT` |
| temperature `signal.out_of_range` | `PHYSICAL_TEMP_ABSOLUTE_LIMIT` |
| SoC `signal.out_of_range` | `PHYSICAL_SOC_RANGE` |
| temperature `signal.stuck` | `SIGNAL_STUCK` |
| `transport.delay/drop`, `source.dropout` | `STREAM_STALE` |
| `signal.combination` | zero, one, or multiple detections |

The Guardian must never infer the injected class.

---

## 11. Compact configuration

```yaml
guardian:
  evaluation_period_ms: 100
  missing_packet_timeout_ms: 500

  warning:
    utilization_threshold: 0.8

  temperature:
    absolute_min_c: -30.0
    absolute_max_c: 70.0
    warning_margin_c: 10.0
    reference_c: 20.0
    hot_state_c: 70.0

    spread:
      cold_c: 12.0
      hot_c: 7.0

    hotspot:
      cold_c: 5.0
      hot_c: 2.0

    dynamics:
      heating_rate_c_per_s:
        cold: 8.0
        hot: 5.0
      cooling_rate_c_per_s: 6.0
      soc_coupling:
        enabled: false
        gain_c_per_pp: 0.25
        rate_cap_pp_per_s: 2.0

  soc:
    min_percent: 0.0
    max_percent: 100.0
    max_rate_pp_per_s: 5.0

  stuck:
    enabled: true
    window_samples: 10
    flatness_epsilon_c: 0.25
    temperature_excitation_c: 1.0
    soc_excitation_pp: 1.0
```
