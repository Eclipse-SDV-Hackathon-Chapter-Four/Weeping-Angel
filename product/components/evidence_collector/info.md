# Evidence Collector — rough outline

Compares what the Guardian reports on uProtocol against the ground truth of one
campaign case (a mutated `.asc` replay plus its `.json` sidecar).

## Inputs

Started with a **case suffix**; it derives both file names from it:

```
evidence_collector --case out_of_range --dir <campaign dir> --zenoh tcp/127.0.0.1:7447
  -> <dir>/battery_temp_out_of_range.asc     replay that is fed into the CAN provider
  -> <dir>/battery_temp_out_of_range.json    what was injected, and when
```

Sidecar `.json` (one entry per injection, timestamps relative to ASC t = 0):

```json
[
  { "timestamp": 40000, "failure": "signal.out_of_range" }
]
```

`failure` uses the `injection_classes` names from
[battery_fault_contract.yaml](../interfaces/battery_fault_contract.yaml). The
expected Guardian detection classes come from that file's `mapping` section
(`expected_guardian`, `may_also_raise`).

## Observed: Guardian output on uProtocol

The collector subscribes to one Guardian status topic. **Assumption: the
Guardian publishes one status message per processed sample**, `OK` or the
active detections:

```json
{ "seq": 400, "status": "FAULT",
  "detections": [ { "detection_class": "PHYSICAL_TEMP_ABSOLUTE_LIMIT",
                    "fault_id": "BatteryTempAbsoluteLimit", "state": "ACTIVE" } ],
  "detected_at_ms": 1791300000123 }
```

The contract's `guardian_evidence_event` currently covers transitions only. The
per-sample `OK` message has to be added to the contract (topic URI + payload).

## Algorithm

1. **Parse the ASC.** Keep the timestamp of every frame with ID `0x100`
   (`BatteryTemperature`). Use the real timestamps from the file instead of
   assuming 10 Hz, because mutated replays (drop, delay) have gaps.
2. **Build the expectation per frame.**
   - `expected_frames` = number of frames.
   - Injection frame `k` = index of the first frame with `t >= timestamp`.
     At 10 Hz, 40000 ms is frame 400, not 40.
   - Frames `0 .. k-1` must be `OK`.
   - From frame `k` on, one of `expected_guardian` must appear within a
     detection window (see open points). Classes in `may_also_raise` are
     allowed; any other class is a FAIL.
3. **Listen and count.** Guardian message `i` is compared with frame `i`.
4. **Stop** after `expected_frames` messages, or after a timeout. For
   `source.dropout` and `transport.*`, fewer messages arrive by design; the
   expected `STREAM_STALE` replaces them.
5. **Verdict.**
   - **PASS:** only `OK` before `k`, and an expected detection inside the window.
   - **FAIL:** a fault before `k` (false positive), no expected detection
     (missed), or an unexpected class.
   - **INCONCLUSIVE:** message count doesn't fit, or no Guardian messages at all.
6. **Write `run.json`** with the case, injections, per-frame observations,
   first detection frame and latency, and the verdict.

## Open points

- [ ] **Counting is fragile.** Messages are aligned by counting, but the
  path ASC → CAN provider → Databroker → VSS bridge → Guardian does not
  guarantee 1 frame = 1 message. The demo bridge publishes once *per signal
  update* (up to 4 per frame), and the Databroker may merge updates.
  Proposal: the bridge or Guardian carries a frame sequence number or the
  source timestamp (`source_message_id` / `source_timestamp_ms` are already
  in the contract as optional fields).
- [ ] **Detection window per class.** Not every detection fires at frame `k`.
  `SIGNAL_STUCK` needs N repeats, `STREAM_STALE` needs the timeout, and drift
  needs a trend window. Define `max_detection_frames` per class (in the
  contract or the sidecar).
- [ ] **End of injection.** Does the fault last until the end of the replay, or
  should the sidecar carry `until`, after which `OK` is expected again
  (recovery)?
- [ ] **Run start sync.** The collector must be subscribed before the replay
  starts. The CAN provider runs with `--infinite`, so the replay must run once,
  or the collector must stop after `expected_frames`.
- [ ] **Language.** Proposal: Rust, reusing `up-rust` + `up-transport-zenoh`
  as in the demo.
