*** Settings ***
Documentation     Battery Thermal Guardian — safety evidence via OpenSOVD.
...
...               Each test injects a battery-temperature or transport fault, lets the
...               Guardian report it to the DFM, and verifies the fault is exposed on the
...               OpenSOVD app fault interface (/sovd/v1/apps/battery_guardian/faults).
...               A Markdown evidence report is written in the suite teardown.

Library           SovdFaultLibrary
...                   gateway=%{GATEWAY=http://127.0.0.1:7690}
...                   app_id=%{APP_ID=battery_guardian}
...                   injector=%{INJECTOR=../target/debug/fault_injector}
...                   catalog=%{CATALOG=../diagnostics/catalog/battery_guardian.json}
...                   report=%{REPORT=../reports/evidence_report.md}

Suite Setup       Opensovd Lists All Catalog Faults    5
Suite Teardown    Write Evidence Report
Test Setup        Reset To Clean Baseline

*** Test Cases ***
Baseline Has No Active Faults
    [Documentation]    A safe temperature ramp raises no faults.
    Inject Scenario    nominal
    ${active}=    Wait For Clear    timeout=8
    Record Scenario    Baseline    Nominal temperature ramp (30..44 C)    ${EMPTY}    PASS

Overtemperature Raises Warning And Critical
    [Documentation]    Ramp through 45 C and 55 C -> warning + critical faults.
    Inject Scenario    overtemp
    Wait For Active Faults    BatteryOverTempWarning    BatteryOverTempCritical    timeout=15
    Record Scenario    Overtemperature    Temp ramp through 45 C / 55 C
    ...    BatteryOverTempWarning BatteryOverTempCritical    PASS

Stuck Signal Is Detected
    [Documentation]    A frozen (unchanging) temperature value trips the stuck-signal fault.
    Inject Scenario    stuck
    Wait For Active Faults    BatteryTempSignalStuck    timeout=15
    Record Scenario    Stuck signal    Frozen temperature value (40 C repeated)
    ...    BatteryTempSignalStuck    PASS

Implausible Spike Is Detected
    [Documentation]    An out-of-range spike (300 C) trips the implausibility fault.
    Inject Scenario    spike
    Wait For Active Faults    BatteryTempImplausible    timeout=15
    Record Scenario    Implausible spike    Out-of-range spike (300 C)
    ...    BatteryTempImplausible    PASS

Source Dropout Raises Stale Fault
    [Documentation]    The temperature source stops publishing -> freshness watchdog fires.
    Inject Scenario    stale
    Wait For Active Faults    BatteryTempSignalStale    timeout=15
    Record Scenario    Source dropout    Publisher stops (no fresh samples)
    ...    BatteryTempSignalStale    PASS

Transport Delay Raises Stale Fault
    [Documentation]    A Toxiproxy blackhole on the Zenoh link delays/drops otherwise-valid
    ...                CAN-derived samples -> the Guardian reports the signal as stale.
    Inject Transport Delay
    Wait For Active Faults    BatteryTempSignalStale    timeout=15
    Record Scenario    Transport delay    Zenoh link blackholed (delayed/dropped signal)
    ...    BatteryTempSignalStale    PASS

*** Keywords ***
Reset To Clean Baseline
    [Documentation]    Clear sticky DTC state and re-establish a fresh, fault-free baseline.
    Reset Faults
    Establish Baseline
    Wait For Clear    timeout=8
