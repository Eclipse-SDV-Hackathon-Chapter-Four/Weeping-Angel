use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};

pub const BATTERY_FRAME_ID: u32 = 0x100;
pub const TEMPERATURE_QUANTUM_C: f32 = 0.5;
pub const SOC_QUANTUM_PP: f32 = 0.5;

const TIMESTAMP_BYTES: usize = 4;
const PAYLOAD_BYTES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SignalValues {
    pub temp_min: f32,
    pub temp_avg: f32,
    pub temp_max: f32,
    pub soc: f32,
}

impl SignalValues {
    pub fn get(self, signal: &str) -> f32 {
        match signal {
            "temp_min" => self.temp_min,
            "temp_avg" => self.temp_avg,
            "temp_max" => self.temp_max,
            "soc" => self.soc,
            _ => unreachable!("validated canonical signal"),
        }
    }

    pub fn set(&mut self, signal: &str, value: f32) {
        match signal {
            "temp_min" => self.temp_min = value,
            "temp_avg" => self.temp_avg = value,
            "temp_max" => self.temp_max = value,
            "soc" => self.soc = value,
            _ => unreachable!("validated canonical signal"),
        }
    }
}

pub fn quantize_signal(signal: &str, value: f32) -> Result<f32> {
    let bytes = if signal == "soc" {
        encode_soc(value)?
    } else {
        encode_temperature(value)?
    };
    Ok(if signal == "soc" {
        decode_unsigned(&bytes) * SOC_QUANTUM_PP
    } else {
        decode_temperature(&bytes)
    })
}

#[derive(Debug, Clone)]
pub struct CaseFrame {
    pub record_index: usize,
    pub sequence_index: usize,
    pub arrival_ms: u64,
    pub source_ms: u64,
    pub values: SignalValues,
    pub removed: bool,
}

#[derive(Debug, Clone)]
pub struct AscDocument {
    records: Vec<Record>,
}

#[derive(Debug, Clone)]
enum Record {
    Raw(String),
    Battery(BatteryRecord),
}

#[derive(Debug, Clone)]
struct BatteryRecord {
    original: String,
    token_spans: Vec<(usize, usize)>,
    payload: Vec<u8>,
    arrival_ms: u64,
    original_arrival_ms: u64,
    values: SignalValues,
    original_values: SignalValues,
    removed: bool,
}

impl AscDocument {
    pub fn load(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("read ASC template {}", path.display()))?;
        Self::parse(&contents).with_context(|| format!("parse ASC template {}", path.display()))
    }

    pub fn parse(contents: &str) -> Result<Self> {
        let mut records = Vec::new();
        for (line_number, line) in contents.split_inclusive('\n').enumerate() {
            records.push(parse_record(line, line_number + 1)?);
        }
        if !contents.is_empty() && !contents.ends_with('\n') {
            // split_inclusive already returned the final unterminated line.
        }
        if !records
            .iter()
            .any(|record| matches!(record, Record::Battery(_)))
        {
            bail!("ASC template contains no battery frame 0x100");
        }
        Ok(Self { records })
    }

    pub fn battery_frames(&self) -> Vec<CaseFrame> {
        let mut sequence_index = 0;
        self.records
            .iter()
            .enumerate()
            .filter_map(|(record_index, record)| match record {
                Record::Battery(frame) => {
                    let result = CaseFrame {
                        record_index,
                        sequence_index,
                        arrival_ms: frame.arrival_ms,
                        source_ms: source_timestamp_ms(&frame.payload),
                        values: frame.values,
                        removed: frame.removed,
                    };
                    sequence_index += 1;
                    Some(result)
                }
                Record::Raw(_) => None,
            })
            .collect()
    }

    pub fn apply_frames(&mut self, frames: &[CaseFrame]) -> Result<()> {
        for frame in frames {
            let record = self
                .records
                .get_mut(frame.record_index)
                .context("candidate references an invalid ASC record")?;
            let Record::Battery(record) = record else {
                bail!("candidate references a non-battery ASC record");
            };
            if source_timestamp_ms(&record.payload) != frame.source_ms {
                bail!("candidate attempted to rewrite a source-generation timestamp");
            }
            record.arrival_ms = frame.arrival_ms;
            record.values = quantize_values(frame.values)?;
            record.payload = encode_payload(&record.payload, record.values)?;
            record.removed = frame.removed;
        }
        Ok(())
    }

    pub fn render(&self) -> String {
        let mut output = String::new();
        for record in &self.records {
            match record {
                Record::Raw(line) => output.push_str(line),
                Record::Battery(frame) if !frame.removed => output.push_str(&frame.render()),
                Record::Battery(_) => {}
            }
        }
        output
    }
}

impl BatteryRecord {
    fn render(&self) -> String {
        if self.arrival_ms == self.original_arrival_ms && self.values == self.original_values {
            return self.original.clone();
        }

        let mut replacements = Vec::with_capacity(self.payload.len() + 1);
        replacements.push((
            self.token_spans[0],
            format!("{:.6}", self.arrival_ms as f64 / 1_000.0),
        ));
        for (index, byte) in self.payload.iter().enumerate() {
            replacements.push((self.token_spans[6 + index], format!("{byte:02X}")));
        }
        replacements.sort_by_key(|(span, _)| std::cmp::Reverse(span.0));

        let mut rendered = self.original.clone();
        for ((start, end), replacement) in replacements {
            rendered.replace_range(start..end, &replacement);
        }
        rendered
    }
}

fn parse_record(line: &str, line_number: usize) -> Result<Record> {
    let spans = token_spans(line);
    if spans.len() < 6 {
        return Ok(Record::Raw(line.to_owned()));
    }
    let token = |index: usize| &line[spans[index].0..spans[index].1];
    let Ok(frame_id) = u32::from_str_radix(token(2), 16) else {
        return Ok(Record::Raw(line.to_owned()));
    };
    if frame_id != BATTERY_FRAME_ID || token(4) != "d" {
        return Ok(Record::Raw(line.to_owned()));
    }

    let dlc = usize::from_str_radix(token(5), 16)
        .with_context(|| format!("line {line_number}: invalid hexadecimal DLC"))?;
    if dlc < PAYLOAD_BYTES {
        bail!(
            "line {line_number}: battery payload has {dlc} bytes; timestamped canonical frame requires at least {PAYLOAD_BYTES}"
        );
    }
    if spans.len() < 6 + dlc {
        bail!("line {line_number}: DLC declares {dlc} bytes but line is shorter");
    }

    let mut payload = Vec::with_capacity(dlc);
    for index in 0..dlc {
        payload.push(
            u8::from_str_radix(token(6 + index), 16)
                .with_context(|| format!("line {line_number}: invalid payload byte {index}"))?,
        );
    }
    let arrival_seconds: f64 = token(0)
        .parse()
        .with_context(|| format!("line {line_number}: invalid ASC timestamp"))?;
    if !arrival_seconds.is_finite() || arrival_seconds < 0.0 {
        bail!("line {line_number}: ASC timestamp must be finite and non-negative");
    }
    let arrival_ms = (arrival_seconds * 1_000.0).round() as u64;
    let source_ms = source_timestamp_ms(&payload);
    if source_ms > u32::MAX as u64 {
        bail!("line {line_number}: source timestamp exceeds 32-bit field");
    }
    let values = decode_values(&payload);
    Ok(Record::Battery(BatteryRecord {
        original: line.to_owned(),
        token_spans: spans,
        payload,
        arrival_ms,
        original_arrival_ms: arrival_ms,
        values,
        original_values: values,
        removed: false,
    }))
}

fn token_spans(line: &str) -> Vec<(usize, usize)> {
    let bytes = line.as_bytes();
    let mut result = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index == bytes.len() {
            break;
        }
        let start = index;
        while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        result.push((start, index));
    }
    result
}

fn source_timestamp_ms(payload: &[u8]) -> u64 {
    u32::from_le_bytes(payload[..TIMESTAMP_BYTES].try_into().expect("four bytes")) as u64
}

fn decode_values(payload: &[u8]) -> SignalValues {
    SignalValues {
        temp_avg: decode_temperature(&payload[4..6]),
        temp_max: decode_temperature(&payload[6..8]),
        temp_min: decode_temperature(&payload[8..10]),
        soc: decode_unsigned(&payload[10..12]) * SOC_QUANTUM_PP,
    }
}

fn decode_temperature(bytes: &[u8]) -> f32 {
    decode_unsigned(bytes) * TEMPERATURE_QUANTUM_C - 40.0
}

fn decode_unsigned(bytes: &[u8]) -> f32 {
    u16::from_le_bytes(bytes.try_into().expect("two bytes")) as f32
}

fn encode_payload(original: &[u8], values: SignalValues) -> Result<Vec<u8>> {
    let mut payload = original.to_vec();
    payload[4..6].copy_from_slice(&encode_temperature(values.temp_avg)?);
    payload[6..8].copy_from_slice(&encode_temperature(values.temp_max)?);
    payload[8..10].copy_from_slice(&encode_temperature(values.temp_min)?);
    payload[10..12].copy_from_slice(&encode_soc(values.soc)?);
    Ok(payload)
}

fn encode_temperature(value: f32) -> Result<[u8; 2]> {
    encode_quantized(value, -40.0, 125.0, TEMPERATURE_QUANTUM_C, "temperature")
}

fn encode_soc(value: f32) -> Result<[u8; 2]> {
    encode_quantized(value, 0.0, 100.0, SOC_QUANTUM_PP, "SoC")
}

fn encode_quantized(
    value: f32,
    minimum: f32,
    maximum: f32,
    quantum: f32,
    name: &str,
) -> Result<[u8; 2]> {
    if !value.is_finite() || value < minimum || value > maximum {
        bail!("{name} value {value} is outside DBC range [{minimum}, {maximum}]");
    }
    let raw = ((value - minimum) / quantum).round();
    let represented = minimum + raw * quantum;
    if (represented - value).abs() > 1e-4 {
        bail!("{name} value {value} is not representable with quantum {quantum}");
    }
    Ok((raw as u16).to_le_bytes())
}

fn quantize_values(values: SignalValues) -> Result<SignalValues> {
    let payload = encode_payload(&[0; PAYLOAD_BYTES], values)?;
    Ok(decode_values(&payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASC: &str = "header\n   0.000000 1  100             Rx   d 10 00 00 00 00 8C 00 90 00 82 00 A0 00 00 00 00 00\nfooter\n";

    #[test]
    fn unmodified_document_round_trips_byte_exactly() {
        let document = AscDocument::parse(ASC).unwrap();
        assert_eq!(document.render(), ASC);
    }

    #[test]
    fn changing_signal_preserves_source_timestamp_and_other_lines() {
        let mut document = AscDocument::parse(ASC).unwrap();
        let mut frames = document.battery_frames();
        frames[0].values.temp_max = 33.0;
        document.apply_frames(&frames).unwrap();
        let rendered = document.render();
        assert!(rendered.starts_with("header\n"));
        assert!(rendered.ends_with("footer\n"));
        let reparsed = AscDocument::parse(&rendered).unwrap();
        let frame = &reparsed.battery_frames()[0];
        assert_eq!(frame.source_ms, 0);
        assert_eq!(frame.values.temp_max, 33.0);
    }

    #[test]
    fn source_timestamp_is_little_endian_and_padding_is_preserved() {
        let asc = "   0.100000 1  100             Rx   d 10 78 56 34 12 8D 00 91 00 83 00 A0 00 DE AD BE EF\n";
        let mut document = AscDocument::parse(asc).unwrap();
        let mut frames = document.battery_frames();
        assert_eq!(frames[0].source_ms, 0x1234_5678);

        frames[0].values.temp_avg = 34.0;
        document.apply_frames(&frames).unwrap();
        assert!(document.render().ends_with("DE AD BE EF\n"));
    }

    #[test]
    fn unrepresentable_value_is_rejected() {
        assert!(encode_temperature(20.25).is_err());
        assert!(encode_soc(100.5).is_err());
    }
}
