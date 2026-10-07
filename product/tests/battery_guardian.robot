*** Settings ***
Documentation     Battery Guardian — physical-consistency and fault-campaign evidence via OpenSOVD.
...
...               CLASSIFICATION CONTRACT
...               =======================
...               The campaign MUST keep injected cause and observed symptom separate.
...
...               1) Campaign / injector -> Evidence Collector (ground truth)
...                  injected_class, run_id, injection_id, injected_at_ms.
...
...               2) Guardian -> Evidence Collector (independent local evidence)
...                  The Guardian MUST emit a GuardianEvidenceEvent before/independently of an
...                  optional DFM write. Required fields: detection_class, detection_level, state, detected_at_ms,
...                  temp_min, temp_avg, temp_max, soc; where applicable also observed, limit,
...                  residual, utilization, signal, source_message_id,
...                  source_timestamp_ms / sequence. Include fault_id only when a configured
...                  DetectionClass x DetectionLevel DFM projection exists.
...
...               3) Evidence Collector
...                  Correlates injection ground truth, direct GuardianEvidenceEvent(s), and
...                  DFM/OpenSOVD visibility. It MUST NOT infer the injected class from a
...                  Guardian detection alone.
...                  In particular, source dropout, transport drop and sufficiently long transport
...                  delay can all appear at the Guardian as STREAM_STALE.
...
...               Guardian observations used by this suite:
...                 THERMAL_LIMIT / WARNING
...                 THERMAL_LIMIT / CRITICAL
...                 PHYSICAL_TEMP_ABSOLUTE_LIMIT / VIOLATION
...                 PHYSICAL_TEMP_ORDERING / VIOLATION
...                 PHYSICAL_TEMP_SPREAD / WARNING or VIOLATION
...                 PHYSICAL_TEMP_HOTSPOT / WARNING or VIOLATION
...                 PHYSICAL_TEMP_RATE / WARNING or VIOLATION
...                 PHYSICAL_SOC_RANGE / VIOLATION
...                 PHYSICAL_SOC_RATE / VIOLATION
...                 SIGNAL_STUCK / VIOLATION
...                 STREAM_STALE / VIOLATION
...                 STREAM_GENERATION_GAP / VIOLATION (missing source-timestamp generations)
...               Continuous WARNING observations remain internal when no DFM mapping exists.
...
...               Physical model checked by the Guardian:
...                 T_abs_min <= T_min <= T_avg <= T_max <= T_abs_max
...                 T_max - T_min <= spread_limit(T_avg)
...                 T_max - T_avg <= hotspot_limit(T_avg)
...                 -rate_down <= dT/dt <= rate_up(T_avg, abs(dSoC/dt))
...                 0 <= SoC <= 100 and abs(dSoC) <= configured per-sample limit
...
...               NOTE: The DBC/VSS stream is sampled every 100 ms. DBC representable ranges
...               are encoding limits, not the physical battery limits used by the Guardian.

Library           SovdFaultLibrary
...                   gateway=%{GATEWAY=http://127.0.0.1:7690}
...                   app_id=%{APP_ID=battery_guardian}
...                   injector=%{INJECTOR=../target/debug/fault_injector}
...                   catalog=%{CATALOG=../config/battery_guardian/guardian_diagnostics.json}
...                   report=%{REPORT=../reports/evidence_report.md}

Suite Setup       Opensovd Lists All Catalog Faults    12
Suite Teardown    Write Evidence Report
Test Setup        Reset To Clean Baseline

*** Variables ***
# DFM/OpenSOVD fault IDs. These are Guardian OBSERVATIONS, not injected causes.
${F_THERMAL_WARNING}     BatteryOverTempWarning
${F_THERMAL_CRITICAL}    BatteryOverTempCritical
${F_TEMP_ABSOLUTE}       BatteryTempAbsoluteLimit
${F_TEMP_ORDERING}       BatteryTempOrdering
${F_TEMP_SPREAD}         BatteryTempSpread
${F_TEMP_HOTSPOT}        BatteryTempHotspot
${F_TEMP_RATE}           BatteryTempRate
${F_SOC_RANGE}           BatterySocRange
${F_SOC_RATE}            BatterySocRate
${F_SIGNAL_STUCK}        BatterySignalStuck
${F_STREAM_STALE}        BatteryTempStreamStale
${F_GENERATION_GAP}      BatteryTempGenerationGap

*** Test Cases ***
# -----------------------------------------------------------------------------
# Physical-model oracle coverage
# -----------------------------------------------------------------------------

Baseline Satisfies Physical Model
    [Documentation]    Nominal min/avg/max/SoC trajectory satisfies every physical invariant.
    [Tags]    baseline    oracle:physical
    Inject Scenario    nominal
    ${active}=    Wait For Clear    timeout=8
    Record Scenario    Baseline    Nominal physically consistent battery trajectory    ${EMPTY}    PASS

Absolute Temperature Limit Is Detected
    [Documentation]    A sample crosses the configured physical battery temperature limit while remaining decodable.
    [Tags]    oracle:physical    detect:PHYSICAL_TEMP_ABSOLUTE_LIMIT
    Inject Scenario    physical_temp_absolute_limit
    Wait For Active Faults    ${F_TEMP_ABSOLUTE}    timeout=15
    Record Scenario    Physical absolute temperature limit    Temperature outside configured safe battery range
    ...    ${F_TEMP_ABSOLUTE}    PASS

Temperature Ordering Violation Is Detected
    [Documentation]    Violates T_min <= T_avg <= T_max without relying on an absolute range violation.
    [Tags]    oracle:physical    detect:PHYSICAL_TEMP_ORDERING
    Inject Scenario    physical_temp_ordering
    Wait For Active Faults    ${F_TEMP_ORDERING}    timeout=15
    Record Scenario    Temperature ordering    min/avg/max ordering violated
    ...    ${F_TEMP_ORDERING}    PASS

Temperature Spread Violation Is Detected
    [Documentation]    T_max-T_min exceeds the temperature-dependent spread_limit(T_avg).
    [Tags]    oracle:physical    detect:PHYSICAL_TEMP_SPREAD
    Inject Scenario    physical_temp_spread
    Wait For Active Faults    ${F_TEMP_SPREAD}    timeout=15
    Record Scenario    Pack temperature spread    max-min exceeds temperature-dependent spread limit
    ...    ${F_TEMP_SPREAD}    PASS

Hotspot Deviation Is Detected
    [Documentation]    T_max-T_avg exceeds hotspot_limit(T_avg); the hottest cell diverges too far from the pack mean.
    [Tags]    oracle:physical    detect:PHYSICAL_TEMP_HOTSPOT
    Inject Scenario    physical_temp_hotspot
    Wait For Active Faults    ${F_TEMP_HOTSPOT}    timeout=15
    Record Scenario    Hotspot deviation    max-average exceeds temperature-dependent hotspot limit
    ...    ${F_TEMP_HOTSPOT}    PASS

Temperature Rate Violation Is Detected
    [Documentation]    A temperature change exceeds the configured 100-ms dynamic bound.
    [Tags]    oracle:physical    detect:PHYSICAL_TEMP_RATE
    Inject Scenario    physical_temp_rate
    Wait For Active Faults    ${F_TEMP_RATE}    timeout=15
    Record Scenario    Temperature rate    dT/dt exceeds temperature/load-dependent rate bound
    ...    ${F_TEMP_RATE}    PASS

SoC Range Violation Is Detected
    [Documentation]    State of charge is outside 0..100 percent.
    [Tags]    oracle:physical    detect:PHYSICAL_SOC_RANGE
    Inject Scenario    physical_soc_range
    Wait For Active Faults    ${F_SOC_RANGE}    timeout=15
    Record Scenario    SoC range    SoC outside physical percentage range
    ...    ${F_SOC_RANGE}    PASS

SoC Rate Violation Is Detected
    [Documentation]    State of charge changes faster than the configured per-sample bound.
    [Tags]    oracle:physical    detect:PHYSICAL_SOC_RATE
    Inject Scenario    physical_soc_rate
    Wait For Active Faults    ${F_SOC_RATE}    timeout=15
    Record Scenario    SoC rate    SoC step exceeds configured 100-ms limit
    ...    ${F_SOC_RATE}    PASS

# -----------------------------------------------------------------------------
# Signal faults: injected cause -> expected Guardian observation(s)
# -----------------------------------------------------------------------------

Signal Stuck Is Detected
    [Documentation]    Fresh messages continue but a signal is frozen. New message identities distinguish this from transport duplication.
    [Tags]    inject:signal.stuck    detect:SIGNAL_STUCK    owner:guardian
    Inject Scenario    signal_stuck
    Wait For Active Faults    ${F_SIGNAL_STUCK}    timeout=15
    Record Scenario    signal.stuck    Fresh messages with frozen signal value
    ...    ${F_SIGNAL_STUCK}    PASS

Signal Spike Is Detected By Physical Consistency
    [Documentation]    In-range spike is chosen so the primary oracle is dynamic/pack consistency, not the DBC encoding range.
    [Tags]    inject:signal.spike    detect:PHYSICAL_TEMP_RATE    owner:guardian
    Inject Scenario    signal_spike
    Wait For Active Faults    ${F_TEMP_RATE}    timeout=15
    Record Scenario    signal.spike    Abrupt in-range temperature spike
    ...    ${F_TEMP_RATE}    PASS

Signal Drift Is Detected By Pack Consistency
    [Documentation]    Slow drift stays within the per-sample rate bound but eventually violates spread/hotspot consistency.
    [Tags]    inject:signal.drift    detect:PHYSICAL_TEMP_SPREAD    detect:PHYSICAL_TEMP_HOTSPOT    owner:guardian
    Inject Scenario    signal_drift
    Wait For Active Faults    ${F_TEMP_SPREAD}    ${F_TEMP_HOTSPOT}    timeout=20
    Record Scenario    signal.drift    Slow hottest-cell drift relative to pack
    ...    ${F_TEMP_SPREAD} ${F_TEMP_HOTSPOT}    PASS

Signal Out Of Range Is Detected
    [Documentation]    Signal crosses the Guardian physical limit. This is distinct from merely exceeding the DBC nominal range.
    [Tags]    inject:signal.out_of_range    detect:PHYSICAL_TEMP_ABSOLUTE_LIMIT    owner:guardian
    Inject Scenario    signal_out_of_range
    Wait For Active Faults    ${F_TEMP_ABSOLUTE}    timeout=15
    Record Scenario    signal.out_of_range    Temperature outside configured physical limit
    ...    ${F_TEMP_ABSOLUTE}    PASS

# -----------------------------------------------------------------------------
# Transport faults
# -----------------------------------------------------------------------------

Transport Delay Raises Stream Stale
    [Documentation]    Delay beyond the freshness deadline is observable as STREAM_STALE; Guardian cannot prove transport.delay as root cause.
    [Tags]    inject:transport.delay    detect:STREAM_STALE    owner:guardian    ambiguous-root-cause
    Inject Transport Delay
    Wait For Active Faults    ${F_STREAM_STALE}    timeout=15
    Record Scenario    transport.delay    Zenoh delay beyond freshness deadline
    ...    ${F_STREAM_STALE}    PASS

Transport Drop Raises Stream Stale
    [Documentation]    Sustained message loss is observable as STREAM_STALE; indistinguishable from source dropout at the receiver alone.
    [Tags]    inject:transport.drop    detect:STREAM_STALE    owner:guardian    ambiguous-root-cause
    Inject Scenario    transport_drop
    Wait For Active Faults    ${F_STREAM_STALE}    timeout=15
    Record Scenario    transport.drop    Sustained transport message loss
    ...    ${F_STREAM_STALE}    PASS

# -----------------------------------------------------------------------------
# Source faults
# -----------------------------------------------------------------------------

Source Dropout Raises Stream Stale
    [Documentation]    Publisher stops. Receiver-side Guardian observes STREAM_STALE, not the root cause source.dropout.
    [Tags]    inject:source.dropout    detect:STREAM_STALE    owner:guardian    ambiguous-root-cause
    Inject Scenario    source_dropout
    Wait For Active Faults    ${F_STREAM_STALE}    timeout=15
    Record Scenario    source.dropout    Publisher stops producing fresh samples
    ...    ${F_STREAM_STALE}    PASS

*** Keywords ***
Reset To Clean Baseline
    [Documentation]    Clear sticky DTC state and re-establish a fresh, fault-free baseline.
    Reset Faults
    Establish Baseline
    Wait For Clear    timeout=8
