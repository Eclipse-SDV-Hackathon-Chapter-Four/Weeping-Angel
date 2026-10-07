# Battery CAN FD Replay Contract

> **Status:** normative specification of the implemented product replay format.
>
> This document captures the format implemented by the canonical ASC asset,
> the Case Mutator ASC parser/renderer, and the KUKSA dump-file replay path.

## 1. Product frame

The product battery message is a CAN FD frame with these fixed properties:

| Property | Value |
|---|---|
| arbitration ID | standard 11-bit ID `0x100` |
| payload length | 16 bytes |
| CAN FD DLC code | `0xA` |
| nominal generation period | 100 ms |
| byte order of defined signals | little endian |
| bit-rate switch (BRS) in the canonical replay | disabled (`0`) |
| error-state indicator (ESI) in the canonical replay | `0` |

The distinction between DLC code and payload length is intentional: the CAN FD
DLC code `0xA` represents a payload length of 16 bytes.

## 2. Payload layout

The payload layout implemented by `product/config/battery_temp.dbc` and the
Case Mutator is:

| Bytes | DBC signal | Encoding |
|---:|---|---|
| 0–3 | `TimeStamp` | unsigned 32-bit milliseconds, little endian |
| 4–5 | `CellTempAvg` | unsigned 16-bit raw, factor 0.5, offset -40 °C |
| 6–7 | `CellTempMax` | unsigned 16-bit raw, factor 0.5, offset -40 °C |
| 8–9 | `CellTempMin` | unsigned 16-bit raw, factor 0.5, offset -40 °C |
| 10–11 | `StateOfCharge` | unsigned 16-bit raw, factor 0.5 pp |
| 12–15 | reserved | uninterpreted and preserved unchanged |

`TimeStamp` starts at `0` for a replay and carries source-relative generation
time. Value mutation, frame deletion, and transport-delay scheduling must not
rewrite the timestamp of a retained frame.

## 3. Canonical Vector ASC representation

Each product battery record uses the CAN FD form emitted and consumed by
`python-can`:

```text
<asc-time> CANFD <channel> Rx <frame-id> <brs> <esi> <dlc-code> <data-length> <payload...> <trailer...>
```

The canonical product form is:

```text
0.100000 CANFD 1 Rx 100 0 0 a 16 64 00 00 00 8D 00 91 00 83 00 A0 00 00 00 00 00 <trailer...>
```

Whitespace is not semantically significant to parsing, but the Mutator
preserves the original line layout except for fields it deliberately changes.
The canonical asset is
`product/config/battery_temp_with_ts.asc`.

## 4. Case Mutator behavior

The implemented parser recognizes two Vector ASC record shapes:

```text
Classic CAN: <time> <channel> <id> Rx d <hex-length> <payload...>
CAN FD:      <time> CANFD <channel> Rx <id> <brs> <esi> <dlc-code> <decimal-length> <payload...>
```

For CAN FD records, the Mutator reads the decimal data-length field and starts
the payload at the following token. For the canonical product frame, that
length is 16 bytes. It preserves the CAN FD marker, BRS, ESI, DLC-code token,
trailer fields, reserved payload bytes, and all non-target lines.

Classic-CAN parsing remains supported for compatibility with existing ASC
inputs. It is not a valid end-to-end representation of the 16-byte product
frame: the product template and every generated product/harness replay must use
the CAN FD form above.

## 5. Harness requirements

Every Golden Scenario and generated experiment replay must:

- use only CAN FD records for product frame `0x100`;
- use DLC code `0xA` and data length 16;
- contain exactly 16 payload bytes per product frame;
- retain the little-endian payload layout from section 2;
- preserve bytes 12–15 unless a later specification assigns them meaning;
- remain readable by the KUKSA CAN Provider dump-file path;
- remain parseable and renderable by the Case Mutator without changing
  non-target content.

Classic 8-byte demo assets remain non-authoritative historical references and
are outside this product replay contract.
