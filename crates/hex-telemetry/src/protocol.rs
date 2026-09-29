//! Packet protocols and the framing state machine.
//!
//! Two formats are supported:
//!
//! * **CSV line mode**: one sample per line of comma-separated numbers, which is
//!   what a hobby sketch printed with `Serial.print` produces. A line has no
//!   integrity check, so a malformed line is rejected by field count and value
//!   range rather than by a checksum.
//! * **Binary framed mode**: a synchronised frame with a length, a sequence
//!   counter, a device timestamp, and a CRC32. Arbitrary binary data is never
//!   parsed without a frame boundary and an integrity check.
//!
//! The decoder is a pure state machine over bytes. It never touches a port, so
//! every framing rule is testable by feeding it a byte script.

use hex_core::{Real, Timebase, TimestampUnit};
use serde::{Deserialize, Serialize};

/// The wire format a device uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "format", rename_all = "snake_case")]
pub enum PacketFormat {
    /// One comma-separated sample per line.
    CsvLine {
        /// Optional header line naming the fields.
        header: Option<Vec<String>>,
        /// Maximum characters accepted in one line before it is discarded.
        maximum_line_length: usize,
        /// Unit of the first field, which is taken as the timestamp.
        timestamp_unit: TimestampUnit,
    },
    /// A synchronised binary frame.
    BinaryFramed(BinaryFrameSpec),
}

impl Default for PacketFormat {
    fn default() -> Self {
        PacketFormat::CsvLine {
            header: None,
            maximum_line_length: 512,
            timestamp_unit: TimestampUnit::Milliseconds,
        }
    }
}

impl PacketFormat {
    /// A CSV format with a known field order.
    pub fn csv(header: Vec<String>, timestamp_unit: TimestampUnit) -> Self {
        PacketFormat::CsvLine {
            header: Some(header),
            maximum_line_length: 512,
            timestamp_unit,
        }
    }

    /// The default binary frame used by the HexaDOF reference firmware.
    pub fn default_binary(field_count: usize) -> Self {
        PacketFormat::BinaryFramed(BinaryFrameSpec::hexadof_default(field_count))
    }

    pub fn label(&self) -> &'static str {
        match self {
            PacketFormat::CsvLine { .. } => "CSV line",
            PacketFormat::BinaryFramed(_) => "Binary framed",
        }
    }

    /// Whether the format carries a checksum.
    pub fn has_integrity_check(&self) -> bool {
        matches!(self, PacketFormat::BinaryFramed(_))
    }
}

/// Endianness of the multi-byte fields in a binary frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Endianness {
    Little,
    Big,
}

/// How each payload field is encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldEncoding {
    /// Unsigned 16-bit integer.
    U16,
    /// Unsigned 32-bit integer.
    U32,
    /// Signed 16-bit integer.
    I16,
    /// Signed 32-bit integer.
    I32,
    /// IEEE-754 single precision.
    F32,
    /// IEEE-754 double precision.
    F64,
}

impl FieldEncoding {
    pub fn width(self) -> usize {
        match self {
            FieldEncoding::U16 | FieldEncoding::I16 => 2,
            FieldEncoding::U32 | FieldEncoding::I32 | FieldEncoding::F32 => 4,
            FieldEncoding::F64 => 8,
        }
    }

    /// Decode one value from a little-endian slice.
    pub fn decode_le(self, bytes: &[u8]) -> Option<Real> {
        let w = self.width();
        if bytes.len() < w {
            return None;
        }
        let b = &bytes[..w];
        Some(match self {
            FieldEncoding::U16 => u16::from_le_bytes([b[0], b[1]]) as Real,
            FieldEncoding::I16 => i16::from_le_bytes([b[0], b[1]]) as Real,
            FieldEncoding::U32 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as Real,
            FieldEncoding::I32 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as Real,
            FieldEncoding::F32 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as Real,
            FieldEncoding::F64 => {
                f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
            }
        })
    }

    /// Decode one value from a big-endian slice.
    pub fn decode_be(self, bytes: &[u8]) -> Option<Real> {
        let w = self.width();
        if bytes.len() < w {
            return None;
        }
        let b = &bytes[..w];
        Some(match self {
            FieldEncoding::U16 => u16::from_be_bytes([b[0], b[1]]) as Real,
            FieldEncoding::I16 => i16::from_be_bytes([b[0], b[1]]) as Real,
            FieldEncoding::U32 => u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as Real,
            FieldEncoding::I32 => i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as Real,
            FieldEncoding::F32 => f32::from_be_bytes([b[0], b[1], b[2], b[3]]) as Real,
            FieldEncoding::F64 => {
                f64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
            }
        })
    }

    pub fn decode(self, bytes: &[u8], endianness: Endianness) -> Option<Real> {
        match endianness {
            Endianness::Little => self.decode_le(bytes),
            Endianness::Big => self.decode_be(bytes),
        }
    }
}

/// One field of a binary frame payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PayloadField {
    /// Name used in the mapping table and the packet inspector.
    pub name: String,
    pub encoding: FieldEncoding,
    /// Multiplier applied after decoding, before the channel mapping.
    pub scale: Real,
    /// Offset added after scaling.
    pub offset: Real,
}

impl PayloadField {
    pub fn new(name: impl Into<String>, encoding: FieldEncoding) -> Self {
        Self {
            name: name.into(),
            encoding,
            scale: 1.0,
            offset: 0.0,
        }
    }

    /// A raw sensor field that needs a scale factor to reach SI units.
    pub fn scaled(name: impl Into<String>, encoding: FieldEncoding, scale: Real) -> Self {
        Self {
            name: name.into(),
            encoding,
            scale,
            offset: 0.0,
        }
    }

    pub fn width(&self) -> usize {
        self.encoding.width()
    }

    /// Apply the field's own scale and offset.
    pub fn interpret(&self, raw: Real) -> Real {
        raw * self.scale + self.offset
    }
}

/// The exact byte layout of a binary frame.
///
/// Every field is described, including the ones a given device does not use, so
/// the decoder can verify a frame rather than guess at it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BinaryFrameSpec {
    /// Two-byte synchronisation pattern that starts every frame.
    pub sync: [u8; 2],
    /// Protocol version byte.
    pub version: u8,
    /// Endianness of every multi-byte field.
    pub endianness: Endianness,
    /// Width of the message type field, or `None` when the device omits it.
    pub message_type: Option<FieldEncoding>,
    /// Width of the sequence counter, or `None` when the device omits it.
    pub sequence: Option<FieldEncoding>,
    /// Width of the device timestamp, or `None` when the device omits it.
    pub device_timestamp: Option<FieldEncoding>,
    /// Units of the device timestamp.
    pub timestamp_unit: TimestampUnit,
    /// Width of the payload length field.
    pub payload_length: FieldEncoding,
    /// Maximum payload bytes accepted, which bounds the resynchronisation cost.
    pub maximum_payload: usize,
    /// CRC algorithm used over the frame.
    pub checksum: ChecksumSpec,
    /// Payload fields in wire order.
    pub fields: Vec<PayloadField>,
}

/// Which checksum a framed protocol uses and what bytes it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecksumSpec {
    /// No integrity check. Only acceptable for a development sketch.
    None,
    /// CRC32 over the frame from the sync bytes to the end of the payload,
    /// stored little endian.
    Crc32LittleEndian,
    /// CRC32 over the same range, stored big endian.
    Crc32BigEndian,
    /// Exclusive-or of every byte from the message type to the end of the
    /// payload, one byte.
    Xor8,
}

impl ChecksumSpec {
    pub fn width(self) -> usize {
        match self {
            ChecksumSpec::None => 0,
            ChecksumSpec::Crc32LittleEndian | ChecksumSpec::Crc32BigEndian => 4,
            ChecksumSpec::Xor8 => 1,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ChecksumSpec::None => "None",
            ChecksumSpec::Crc32LittleEndian => "CRC32, little endian",
            ChecksumSpec::Crc32BigEndian => "CRC32, big endian",
            ChecksumSpec::Xor8 => "XOR8",
        }
    }

    /// Compute the checksum bytes for a frame body.
    pub fn compute(self, body: &[u8]) -> Vec<u8> {
        match self {
            ChecksumSpec::None => Vec::new(),
            ChecksumSpec::Crc32LittleEndian => {
                let mut h = crc32fast::Hasher::new();
                h.update(body);
                h.finalize().to_le_bytes().to_vec()
            }
            ChecksumSpec::Crc32BigEndian => {
                let mut h = crc32fast::Hasher::new();
                h.update(body);
                h.finalize().to_be_bytes().to_vec()
            }
            ChecksumSpec::Xor8 => {
                let mut acc = 0u8;
                for b in body {
                    acc ^= *b;
                }
                vec![acc]
            }
        }
    }

    /// Verify a received checksum.
    pub fn verify(self, body: &[u8], received: &[u8]) -> bool {
        self.compute(body) == received
    }
}

impl BinaryFrameSpec {
    /// The reference frame layout.
    ///
    /// `SYNC VERSION MSGTYPE SEQ TIMESTAMP LENGTH PAYLOAD CRC32`, little endian,
    /// with the timestamp as a 32-bit microsecond counter. This is the layout the
    /// shipped firmware example uses and the layout the documentation describes.
    pub fn hexadof_default(field_count: usize) -> Self {
        let names = [
            "ax", "ay", "az", "gx", "gy", "gz", "mx", "my", "mz", "baro", "temp",
        ];
        let fields = (0..field_count)
            .map(|i| {
                let name = names.get(i).copied().unwrap_or("field");
                let name = if i < names.len() {
                    name.to_string()
                } else {
                    format!("field{}", i)
                };
                PayloadField::new(name, FieldEncoding::F32)
            })
            .collect();
        Self {
            sync: [0xAA, 0x55],
            version: 1,
            endianness: Endianness::Little,
            message_type: Some(FieldEncoding::U16),
            sequence: Some(FieldEncoding::U16),
            device_timestamp: Some(FieldEncoding::U32),
            timestamp_unit: TimestampUnit::Microseconds,
            payload_length: FieldEncoding::U16,
            maximum_payload: 256,
            checksum: ChecksumSpec::Crc32LittleEndian,
            fields,
        }
    }

    /// Total payload width implied by the field list.
    pub fn payload_width(&self) -> usize {
        self.fields.iter().map(|f| f.width()).sum()
    }

    /// Fixed bytes in a frame excluding the payload and the checksum.
    pub fn header_width(&self) -> usize {
        let mut w = self.sync.len() + 1; // sync plus version
        for e in [self.message_type, self.sequence, self.device_timestamp]
            .into_iter()
            .flatten()
        {
            w += e.width();
        }
        w + self.payload_length.width()
    }

    /// Total frame size for the declared payload width.
    pub fn frame_width(&self) -> usize {
        self.header_width() + self.payload_width() + self.checksum.width()
    }

    /// Field index for a name.
    pub fn field_index(&self, name: &str) -> Option<usize> {
        self.fields.iter().position(|f| f.name == name)
    }

    /// Verify that the declared layout is self-consistent.
    ///
    /// Returns descriptions of each problem found.
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.sync[0] == self.sync[1] {
            problems.push(
                "the two sync bytes are identical, which makes false syncs likely".to_string(),
            );
        }
        if self.fields.is_empty() {
            problems.push("no payload fields are declared".to_string());
        }
        if self.payload_width() > self.maximum_payload {
            problems.push(format!(
                "the declared payload is {} bytes but the maximum accepted is {}",
                self.payload_width(),
                self.maximum_payload
            ));
        }
        if self.payload_length.width() < 2 {
            problems.push("the payload length field is too narrow to describe a frame".to_string());
        }
        if self.device_timestamp.is_some() && self.timestamp_unit.to_seconds_factor().is_none() {
            problems.push("the timestamp unit needs a tick rate to be convertible".to_string());
        }
        let mut seen = std::collections::HashSet::new();
        for f in &self.fields {
            if !seen.insert(f.name.clone()) {
                problems.push(format!("duplicate payload field name {}", f.name));
            }
        }
        problems
    }
}

/// A successfully decoded packet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecodedPacket {
    /// Device timestamp converted to seconds, when the frame carried one.
    pub device_time: Option<Real>,
    /// Host arrival time in seconds, filled in by the pipeline.
    pub host_time: Option<Real>,
    /// Message type value, when the frame carried one.
    pub message_type: Option<u32>,
    /// Sequence counter, when the frame carried one.
    pub sequence: Option<u32>,
    /// Field values in the order the field list declares, after the field's own
    /// scale and offset and before the channel mapping.
    pub values: Vec<Real>,
    /// Field names matching `values`, so a mapping stays readable in a log.
    pub field_names: Vec<String>,
    /// Frame size in bytes, for the byte-rate display.
    pub frame_bytes: usize,
    /// Raw payload bytes, kept for the packet inspector.
    pub raw_payload: Vec<u8>,
}

impl DecodedPacket {
    /// Look up a field by name.
    pub fn value(&self, name: &str) -> Option<Real> {
        self.field_names
            .iter()
            .position(|n| n == name)
            .and_then(|i| self.values.get(i).copied())
    }
}

/// Why a frame was rejected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FrameRejection {
    /// The checksum did not match.
    ChecksumMismatch {
        expected: Vec<u8>,
        found: Vec<u8>,
        sequence: Option<u32>,
    },
    /// The declared payload length exceeded the configured maximum.
    PayloadTooLong { declared: usize, maximum: usize },
    /// The protocol version is not the one configured.
    VersionMismatch { found: u8, expected: u8 },
    /// The payload length did not match the declared field layout.
    PayloadWidthMismatch { declared: usize, expected: usize },
    /// A CSV line had the wrong number of fields.
    FieldCountMismatch { found: usize, expected: usize },
    /// A CSV line could not be parsed as numbers.
    NotNumeric { field_index: usize, token: String },
    /// A CSV line was longer than the configured maximum.
    LineTooLong { length: usize, maximum: usize },
    /// A CSV line contained a value that was not finite.
    NonFinite { field_index: usize },
}

impl FrameRejection {
    /// A one-line explanation for the warning strip.
    pub fn detail(&self) -> String {
        match self {
            FrameRejection::ChecksumMismatch { sequence, .. } => match sequence {
                Some(s) => format!("Checksum failure on packet {}. The stream may be corrupt.", s),
                None => "Checksum failure. The stream may be corrupt.".to_string(),
            },
            FrameRejection::PayloadTooLong { declared, maximum } => format!(
                "A frame declared {} payload bytes, above the configured maximum of {}. The baud rate or packet format is probably wrong.",
                declared, maximum
            ),
            FrameRejection::VersionMismatch { found, expected } => format!(
                "Frame version {} does not match the configured version {}. Update the device profile.",
                found, expected
            ),
            FrameRejection::PayloadWidthMismatch { declared, expected } => format!(
                "A frame declared {} payload bytes but the field layout needs {}. Check the field list in the device profile.",
                declared, expected
            ),
            FrameRejection::FieldCountMismatch { found, expected } => format!(
                "A line had {} fields but the profile expects {}. The device may be printing a different format.",
                found, expected
            ),
            FrameRejection::NotNumeric { field_index, token } => format!(
                "Field {} could not be read as a number: {}. The line was discarded.",
                field_index, token
            ),
            FrameRejection::LineTooLong { length, maximum } => format!(
                "A line was {} characters, above the {} character limit. The baud rate may be wrong.",
                length, maximum
            ),
            FrameRejection::NonFinite { field_index } => format!(
                "Field {} held a value that is not finite. The line was discarded.",
                field_index
            ),
        }
    }

    /// A stable code for the diagnostics bundle.
    pub fn code(&self) -> &'static str {
        match self {
            FrameRejection::ChecksumMismatch { .. } => "packet.checksum",
            FrameRejection::PayloadTooLong { .. } => "packet.payload_too_long",
            FrameRejection::VersionMismatch { .. } => "packet.version",
            FrameRejection::PayloadWidthMismatch { .. } => "packet.payload_width",
            FrameRejection::FieldCountMismatch { .. } => "packet.field_count",
            FrameRejection::NotNumeric { .. } => "packet.not_numeric",
            FrameRejection::LineTooLong { .. } => "packet.line_too_long",
            FrameRejection::NonFinite { .. } => "packet.non_finite",
        }
    }
}

/// Result of pushing bytes into the decoder.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DecodeOutcome {
    /// Packets decoded from this push.
    pub packets: Vec<DecodedPacket>,
    /// Frames rejected, with a reason each.
    pub rejections: Vec<FrameRejection>,
    /// Bytes consumed from the input.
    pub bytes_consumed: usize,
    /// Bytes discarded while resynchronising.
    pub bytes_discarded: usize,
}

impl DecodeOutcome {
    /// Whether this push produced anything worth reporting.
    pub fn is_empty(&self) -> bool {
        self.packets.is_empty() && self.rejections.is_empty()
    }

    /// Merge another outcome into this one.
    pub fn extend(&mut self, other: DecodeOutcome) {
        self.packets.extend(other.packets);
        self.rejections.extend(other.rejections);
        self.bytes_consumed += other.bytes_consumed;
        self.bytes_discarded += other.bytes_discarded;
    }
}

/// The framing decoder.
///
/// A stateful byte buffer that yields complete packets. It tolerates partial
/// frames across calls and resynchronises after a checksum failure by scanning
/// forward for the next sync pattern, so one corrupted frame does not poison the
/// rest of a session.
#[derive(Debug, Clone)]
pub struct PacketDecoder {
    format: PacketFormat,
    buffer: Vec<u8>,
    /// Sequence of the most recent accepted packet, for gap detection.
    last_sequence: Option<u32>,
    /// Frames discarded because of an integrity failure.
    pub checksum_failures: u64,
    /// Frames discarded for a layout reason.
    pub format_failures: u64,
    /// Sequence gaps observed.
    pub sequence_gaps: u64,
    /// Packets accepted.
    pub packets_accepted: u64,
    /// Bytes discarded while resynchronising.
    pub bytes_discarded: u64,
}

impl PacketDecoder {
    pub fn new(format: PacketFormat) -> Self {
        Self {
            format,
            buffer: Vec::with_capacity(8192),
            last_sequence: None,
            checksum_failures: 0,
            format_failures: 0,
            sequence_gaps: 0,
            packets_accepted: 0,
            bytes_discarded: 0,
        }
    }

    pub fn format(&self) -> &PacketFormat {
        &self.format
    }

    /// The most recent accepted sequence number.
    pub fn last_sequence(&self) -> Option<u32> {
        self.last_sequence
    }

    /// Bytes currently held waiting for the rest of a frame.
    pub fn buffered_bytes(&self) -> usize {
        self.buffer.len()
    }

    /// Discard any buffered partial frame, for example after a reconnect.
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.last_sequence = None;
    }

    /// Feed bytes and collect whatever complete packets result.
    pub fn push(&mut self, bytes: &[u8]) -> DecodeOutcome {
        self.buffer.extend_from_slice(bytes);
        let mut outcome = DecodeOutcome::default();

        match self.format.clone() {
            PacketFormat::CsvLine {
                maximum_line_length,
                timestamp_unit,
                header,
            } => {
                // An empty header list carries no field count information, so it must not
                // be read as a requirement of zero fields.
                let expected = header.as_ref().filter(|h| !h.is_empty()).map(|h| h.len());
                loop {
                    let Some(newline) = self.buffer.iter().position(|b| *b == b'\n') else {
                        if self.buffer.len() > maximum_line_length {
                            let length = self.buffer.len();
                            self.buffer.clear();
                            self.format_failures += 1;
                            outcome.rejections.push(FrameRejection::LineTooLong {
                                length,
                                maximum: maximum_line_length,
                            });
                        }
                        break;
                    };
                    let mut line: Vec<u8> = self.buffer.drain(..=newline).collect();
                    outcome.bytes_consumed += line.len();
                    line.pop();
                    if line.last() == Some(&b'\r') {
                        line.pop();
                    }
                    if line.len() > maximum_line_length {
                        self.format_failures += 1;
                        outcome.rejections.push(FrameRejection::LineTooLong {
                            length: line.len(),
                            maximum: maximum_line_length,
                        });
                        continue;
                    }
                    match decode_csv_line(&line, timestamp_unit, expected) {
                        Ok(Some(packet)) => {
                            self.packets_accepted += 1;
                            outcome.packets.push(packet);
                        }
                        Ok(None) => {
                            // A header or blank line, which is expected and not a
                            // failure.
                        }
                        Err(reason) => {
                            self.format_failures += 1;
                            outcome.rejections.push(reason);
                        }
                    }
                }
            }
            PacketFormat::BinaryFramed(spec) => {
                loop {
                    match find_sync(&self.buffer, spec.sync) {
                        None => {
                            // Keep at most one byte, in case the first sync byte
                            // has already arrived.
                            if self.buffer.len() > 1 {
                                let discarded = self.buffer.len() - 1;
                                self.buffer.drain(..discarded);
                                self.bytes_discarded += discarded as u64;
                                outcome.bytes_discarded += discarded;
                            }
                            break;
                        }
                        Some(0) => {}
                        Some(offset) => {
                            self.buffer.drain(..offset);
                            self.bytes_discarded += offset as u64;
                            outcome.bytes_discarded += offset;
                        }
                    }

                    match self.try_decode_binary(&spec) {
                        BinaryAttempt::NeedMore => break,
                        BinaryAttempt::Accepted(packet, consumed) => {
                            self.buffer.drain(..consumed);
                            outcome.bytes_consumed += consumed;
                            self.packets_accepted += 1;
                            if let Some(seq) = packet.sequence {
                                if let Some(previous) = self.last_sequence {
                                    if seq > previous + 1 {
                                        self.sequence_gaps += 1;
                                    }
                                }
                                self.last_sequence = Some(seq);
                            }
                            outcome.packets.push(packet);
                        }
                        BinaryAttempt::Rejected(reason, consumed) => {
                            // Drop the sync pattern so the scan moves past it,
                            // which is what resynchronisation means in practice.
                            let drop = consumed.min(self.buffer.len()).max(1);
                            self.buffer.drain(..drop);
                            outcome.bytes_consumed += drop;
                            outcome.bytes_discarded += drop;
                            self.bytes_discarded += drop as u64;
                            match reason {
                                FrameRejection::ChecksumMismatch { .. } => {
                                    self.checksum_failures += 1
                                }
                                _ => self.format_failures += 1,
                            }
                            outcome.rejections.push(reason);
                        }
                    }
                }
            }
        }

        outcome
    }

    /// Try to decode one binary frame from the front of the buffer.
    fn try_decode_binary(&self, spec: &BinaryFrameSpec) -> BinaryAttempt {
        let buf = &self.buffer;
        let sync_len = spec.sync.len();
        let header = spec.header_width();

        // Version byte sits immediately after the sync pattern.
        if buf.len() < sync_len + 1 {
            return BinaryAttempt::NeedMore;
        }
        let version = buf[sync_len];
        if version != spec.version {
            return BinaryAttempt::Rejected(
                FrameRejection::VersionMismatch {
                    found: version,
                    expected: spec.version,
                },
                sync_len + 1,
            );
        }

        // Walk the variable header to find the declared payload length.
        let mut cursor = sync_len + 1;
        if let Some(enc) = spec.message_type {
            cursor += enc.width();
        }
        if let Some(enc) = spec.sequence {
            cursor += enc.width();
        }
        if let Some(enc) = spec.device_timestamp {
            cursor += enc.width();
        }
        let length_offset = cursor;
        let length_width = spec.payload_length.width();
        if buf.len() < length_offset + length_width {
            return BinaryAttempt::NeedMore;
        }
        let Some(declared_real) = spec
            .payload_length
            .decode(&buf[length_offset..], spec.endianness)
        else {
            return BinaryAttempt::NeedMore;
        };
        let declared = declared_real.max(0.0) as usize;
        if declared > spec.maximum_payload {
            return BinaryAttempt::Rejected(
                FrameRejection::PayloadTooLong {
                    declared,
                    maximum: spec.maximum_payload,
                },
                sync_len + 1,
            );
        }

        let total = header + declared + spec.checksum.width();
        if buf.len() < total {
            return BinaryAttempt::NeedMore;
        }
        let frame = &buf[..total];
        let payload_start = header;
        let payload_end = header + declared;
        let body = &frame[..payload_end];
        let received_checksum = &frame[payload_end..payload_end + spec.checksum.width()];

        // Decode the header fields before the integrity check so a checksum
        // failure can still name the sequence number.
        let mut cursor = sync_len + 1;
        let mut message_type = None;
        let mut sequence = None;
        let mut device_time = None;
        if let Some(enc) = spec.message_type {
            message_type = enc
                .decode(&frame[cursor..], spec.endianness)
                .map(|v| v.max(0.0) as u32);
            cursor += enc.width();
        }
        if let Some(enc) = spec.sequence {
            sequence = enc
                .decode(&frame[cursor..], spec.endianness)
                .map(|v| v.max(0.0) as u32);
            cursor += enc.width();
        }
        if let Some(enc) = spec.device_timestamp {
            device_time = enc
                .decode(&frame[cursor..], spec.endianness)
                .and_then(|raw| spec.timestamp_unit.to_seconds_factor().map(|f| raw * f));
        }

        if !spec.checksum.verify(body, received_checksum) {
            return BinaryAttempt::Rejected(
                FrameRejection::ChecksumMismatch {
                    expected: spec.checksum.compute(body),
                    found: received_checksum.to_vec(),
                    sequence,
                },
                sync_len + 1,
            );
        }

        let expected_payload = spec.payload_width();
        if declared != expected_payload {
            return BinaryAttempt::Rejected(
                FrameRejection::PayloadWidthMismatch {
                    declared,
                    expected: expected_payload,
                },
                total,
            );
        }

        let mut values = Vec::with_capacity(spec.fields.len());
        let mut field_names = Vec::with_capacity(spec.fields.len());
        let mut offset = payload_start;
        for field in &spec.fields {
            let width = field.width();
            match field
                .encoding
                .decode(&frame[offset..offset + width], spec.endianness)
            {
                Some(raw) => values.push(field.interpret(raw)),
                None => values.push(Real::NAN),
            }
            field_names.push(field.name.clone());
            offset += width;
        }

        BinaryAttempt::Accepted(
            DecodedPacket {
                device_time,
                host_time: None,
                message_type,
                sequence,
                values,
                field_names,
                frame_bytes: total,
                raw_payload: frame[payload_start..payload_end].to_vec(),
            },
            total,
        )
    }
}

/// Outcome of one attempt at a binary frame.
enum BinaryAttempt {
    NeedMore,
    Accepted(DecodedPacket, usize),
    Rejected(FrameRejection, usize),
}

/// Find the first occurrence of a sync pattern.
fn find_sync(buffer: &[u8], sync: [u8; 2]) -> Option<usize> {
    buffer
        .windows(2)
        .position(|w| w[0] == sync[0] && w[1] == sync[1])
}

/// Decode one CSV line into a packet.
///
/// Returns `Ok(None)` for a blank line or a header line, which are not failures.
fn decode_csv_line(
    line: &[u8],
    timestamp_unit: TimestampUnit,
    expected_fields: Option<usize>,
) -> Result<Option<DecodedPacket>, FrameRejection> {
    let text = std::str::from_utf8(line).map_err(|_| FrameRejection::NotNumeric {
        field_index: 0,
        token: "<invalid utf-8>".to_string(),
    })?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.starts_with('#') || trimmed.starts_with("//") {
        return Ok(None);
    }

    let tokens: Vec<&str> = trimmed.split(',').map(|t| t.trim()).collect();

    // A header line is one whose first token is not a number.
    if tokens[0].parse::<Real>().is_err() {
        return Ok(None);
    }

    if let Some(expected) = expected_fields {
        if tokens.len() != expected {
            return Err(FrameRejection::FieldCountMismatch {
                found: tokens.len(),
                expected,
            });
        }
    }

    let mut values = Vec::with_capacity(tokens.len());
    for (i, token) in tokens.iter().enumerate() {
        match token.parse::<Real>() {
            Ok(v) if v.is_finite() => values.push(v),
            Ok(_) => return Err(FrameRejection::NonFinite { field_index: i }),
            Err(_) => {
                return Err(FrameRejection::NotNumeric {
                    field_index: i,
                    token: token.to_string(),
                })
            }
        }
    }

    let device_time = values
        .first()
        .copied()
        .and_then(|raw| timestamp_unit.to_seconds_factor().map(|f| raw * f));

    let field_names = (0..values.len())
        .map(|i| format!("field{}", i))
        .collect::<Vec<_>>();
    let raw_payload = line.to_vec();

    Ok(Some(DecodedPacket {
        device_time,
        host_time: None,
        message_type: None,
        sequence: None,
        values,
        field_names,
        frame_bytes: line.len() + 1,
        raw_payload,
    }))
}

/// Encode a packet using a binary frame specification.
///
/// The application never sends commands to a device in the MVP, but the encoder
/// is needed by the loopback test and by a future firmware generator, so it lives
/// beside the decoder where the two cannot drift apart.
pub fn encode_frame(
    spec: &BinaryFrameSpec,
    sequence: u32,
    device_time_raw: u64,
    values: &[Real],
) -> Option<Vec<u8>> {
    if values.len() != spec.fields.len() {
        return None;
    }
    let mut frame = Vec::with_capacity(spec.frame_width());
    frame.extend_from_slice(&spec.sync);
    frame.push(spec.version);

    let push_u = |frame: &mut Vec<u8>, encoding: FieldEncoding, v: u64| {
        let bytes = match (encoding, spec.endianness) {
            (FieldEncoding::U16, Endianness::Little) => (v as u16).to_le_bytes().to_vec(),
            (FieldEncoding::U16, Endianness::Big) => (v as u16).to_be_bytes().to_vec(),
            (FieldEncoding::U32, Endianness::Little) => (v as u32).to_le_bytes().to_vec(),
            (FieldEncoding::U32, Endianness::Big) => (v as u32).to_be_bytes().to_vec(),
            _ => return,
        };
        frame.extend_from_slice(&bytes);
    };

    if let Some(enc) = spec.message_type {
        push_u(&mut frame, enc, 1);
    }
    if let Some(enc) = spec.sequence {
        push_u(&mut frame, enc, sequence as u64);
    }
    if let Some(enc) = spec.device_timestamp {
        push_u(&mut frame, enc, device_time_raw);
    }
    push_u(&mut frame, spec.payload_length, spec.payload_width() as u64);

    for (field, value) in spec.fields.iter().zip(values.iter()) {
        let raw = if field.scale.abs() > 1e-300 {
            (value - field.offset) / field.scale
        } else {
            *value
        };
        let bytes = match (field.encoding, spec.endianness) {
            (FieldEncoding::F32, Endianness::Little) => (raw as f32).to_le_bytes().to_vec(),
            (FieldEncoding::F32, Endianness::Big) => (raw as f32).to_be_bytes().to_vec(),
            (FieldEncoding::F64, Endianness::Little) => raw.to_le_bytes().to_vec(),
            (FieldEncoding::F64, Endianness::Big) => raw.to_be_bytes().to_vec(),
            (FieldEncoding::I16, Endianness::Little) => (raw as i16).to_le_bytes().to_vec(),
            (FieldEncoding::I16, Endianness::Big) => (raw as i16).to_be_bytes().to_vec(),
            (FieldEncoding::I32, Endianness::Little) => (raw as i32).to_le_bytes().to_vec(),
            (FieldEncoding::I32, Endianness::Big) => (raw as i32).to_be_bytes().to_vec(),
            (FieldEncoding::U16, Endianness::Little) => (raw as u16).to_le_bytes().to_vec(),
            (FieldEncoding::U16, Endianness::Big) => (raw as u16).to_be_bytes().to_vec(),
            (FieldEncoding::U32, Endianness::Little) => (raw as u32).to_le_bytes().to_vec(),
            (FieldEncoding::U32, Endianness::Big) => (raw as u32).to_be_bytes().to_vec(),
        };
        frame.extend_from_slice(&bytes);
    }

    let checksum = spec.checksum.compute(&frame);
    frame.extend_from_slice(&checksum);
    Some(frame)
}

/// Decode a stream of `DecodedPacket`s into a channel table.
///
/// Kept here rather than in the mapping module so the decoder can be tested end
/// to end without a device profile.
pub fn packets_to_columns(packets: &[DecodedPacket]) -> (Vec<Real>, Vec<Vec<Real>>, Vec<String>) {
    let names = packets
        .first()
        .map(|p| p.field_names.clone())
        .unwrap_or_default();
    let mut times = Vec::with_capacity(packets.len());
    let mut columns = vec![Vec::with_capacity(packets.len()); names.len()];
    for p in packets {
        times.push(p.device_time.unwrap_or(Real::NAN));
        for (i, v) in p.values.iter().enumerate() {
            if let Some(col) = columns.get_mut(i) {
                col.push(*v);
            }
        }
    }
    (times, columns, names)
}

/// A timebase implied by a decoded packet stream.
pub fn timebase_for(spec: &PacketFormat) -> Timebase {
    match spec {
        PacketFormat::CsvLine { timestamp_unit, .. } => Timebase {
            source: hex_core::TimeSource::Device,
            unit: *timestamp_unit,
            ..Default::default()
        },
        PacketFormat::BinaryFramed(b) => Timebase {
            source: hex_core::TimeSource::Device,
            unit: b.timestamp_unit,
            note: Some(format!(
                "Binary framed, {}, checksum {}",
                match b.endianness {
                    Endianness::Little => "little endian",
                    Endianness::Big => "big endian",
                },
                b.checksum.label()
            )),
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_spec() -> BinaryFrameSpec {
        BinaryFrameSpec::hexadof_default(3)
    }

    fn push_all(decoder: &mut PacketDecoder, bytes: &[u8], chunk: usize) -> DecodeOutcome {
        let mut total = DecodeOutcome::default();
        for part in bytes.chunks(chunk.max(1)) {
            total.extend(decoder.push(part));
        }
        total
    }

    #[test]
    fn csv_lines_decode_into_packets() {
        let format = PacketFormat::csv(
            vec!["t".into(), "ax".into(), "ay".into()],
            TimestampUnit::Seconds,
        );
        let mut d = PacketDecoder::new(format);
        let out = d.push(b"0.000,1.0,2.0\n0.001,1.1,2.1\n");
        assert_eq!(out.packets.len(), 2);
        assert_eq!(out.packets[0].values, vec![0.0, 1.0, 2.0]);
        assert!((out.packets[1].device_time.unwrap() - 0.001).abs() < 1e-12);
        assert_eq!(d.packets_accepted, 2);
        assert!(out.rejections.is_empty());
    }

    #[test]
    fn csv_header_and_comment_lines_are_skipped_not_rejected() {
        let format = PacketFormat::csv(vec!["t".into(), "ax".into()], TimestampUnit::Seconds);
        let mut d = PacketDecoder::new(format);
        let out = d.push(b"# comment\nt,ax\n0.0,1.0\n\n0.1,2.0\n");
        assert_eq!(out.packets.len(), 2);
        assert!(out.rejections.is_empty());
    }

    #[test]
    fn csv_field_count_mismatch_is_rejected() {
        let format = PacketFormat::csv(
            vec!["t".into(), "ax".into(), "ay".into()],
            TimestampUnit::Seconds,
        );
        let mut d = PacketDecoder::new(format);
        let out = d.push(b"0.0,1.0\n");
        assert!(out.packets.is_empty());
        assert_eq!(out.rejections.len(), 1);
        assert!(matches!(
            out.rejections[0],
            FrameRejection::FieldCountMismatch {
                found: 2,
                expected: 3
            }
        ));
        assert!(out.rejections[0].detail().contains("2 fields"));
    }

    #[test]
    fn csv_non_numeric_field_is_rejected_with_its_index() {
        let format = PacketFormat::csv(vec!["t".into(), "ax".into()], TimestampUnit::Seconds);
        let mut d = PacketDecoder::new(format);
        let out = d.push(b"0.0,abc\n");
        assert_eq!(out.rejections.len(), 1);
        match &out.rejections[0] {
            FrameRejection::NotNumeric { field_index, token } => {
                assert_eq!(*field_index, 1);
                assert_eq!(token, "abc");
            }
            other => panic!("unexpected rejection {:?}", other),
        }
    }

    #[test]
    fn csv_partial_line_waits_for_the_rest() {
        let format = PacketFormat::csv(vec!["t".into(), "ax".into()], TimestampUnit::Seconds);
        let mut d = PacketDecoder::new(format);
        assert!(d.push(b"0.0,1.").packets.is_empty());
        let out = d.push(b"5\n");
        assert_eq!(out.packets.len(), 1);
        assert_eq!(out.packets[0].values, vec![0.0, 1.5]);
    }

    #[test]
    fn csv_overlong_line_is_rejected() {
        let format = PacketFormat::CsvLine {
            header: None,
            maximum_line_length: 16,
            timestamp_unit: TimestampUnit::Seconds,
        };
        let mut d = PacketDecoder::new(format);
        let out = d.push(b"0.0,111111111111111111111111\n");
        assert_eq!(out.rejections.len(), 1);
        assert!(matches!(
            out.rejections[0],
            FrameRejection::LineTooLong { .. }
        ));
    }

    #[test]
    fn csv_carriage_returns_are_tolerated() {
        let format = PacketFormat::csv(vec!["t".into(), "ax".into()], TimestampUnit::Seconds);
        let mut d = PacketDecoder::new(format);
        let out = d.push(b"0.0,1.0\r\n0.1,2.0\r\n");
        assert_eq!(out.packets.len(), 2);
    }

    #[test]
    fn binary_frame_round_trips_through_encode_and_decode() {
        let spec = default_spec();
        let mut d = PacketDecoder::new(PacketFormat::BinaryFramed(spec.clone()));
        let values = [1.5, -2.25, 9.75];
        let frame = encode_frame(&spec, 7, 123_456, &values).unwrap();
        let out = d.push(&frame);
        assert_eq!(out.packets.len(), 1);
        let p = &out.packets[0];
        assert_eq!(p.sequence, Some(7));
        assert_eq!(p.values.len(), 3);
        for (a, b) in p.values.iter().zip(values.iter()) {
            // The field width is f32, so compare at single precision.
            assert!((a - b).abs() < 1e-6, "{} vs {}", a, b);
        }
        assert!((p.device_time.unwrap() - 0.123456).abs() < 1e-12);
    }

    #[test]
    fn binary_decode_survives_arbitrary_split_points() {
        let spec = default_spec();
        let values = [1.0, 2.0, 3.0];
        let frame = encode_frame(&spec, 1, 1000, &values).unwrap();
        for chunk in 1..=frame.len() {
            let mut d = PacketDecoder::new(PacketFormat::BinaryFramed(spec.clone()));
            let out = push_all(&mut d, &frame, chunk);
            assert_eq!(out.packets.len(), 1, "chunk size {}", chunk);
            assert_eq!(out.packets[0].values.len(), 3);
        }
    }

    #[test]
    fn binary_leading_garbage_is_discarded_and_the_frame_still_decodes() {
        let spec = default_spec();
        let frame = encode_frame(&spec, 3, 500, &[1.0, 2.0, 3.0]).unwrap();
        let mut bytes = vec![0x00, 0xAA, 0x00, 0x11, 0x22];
        bytes.extend_from_slice(&frame);
        let mut d = PacketDecoder::new(PacketFormat::BinaryFramed(spec));
        let out = d.push(&bytes);
        assert_eq!(out.packets.len(), 1);
        assert!(out.bytes_discarded > 0);
    }

    #[test]
    fn binary_checksum_failure_is_reported_and_the_stream_resynchronises() {
        let spec = default_spec();
        let good_a = encode_frame(&spec, 1, 100, &[1.0, 2.0, 3.0]).unwrap();
        let mut corrupt = encode_frame(&spec, 2, 200, &[4.0, 5.0, 6.0]).unwrap();
        // Flip a payload byte without touching the checksum.
        corrupt[spec.header_width()] ^= 0xFF;
        let good_b = encode_frame(&spec, 3, 300, &[7.0, 8.0, 9.0]).unwrap();

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&good_a);
        bytes.extend_from_slice(&corrupt);
        bytes.extend_from_slice(&good_b);

        let mut d = PacketDecoder::new(PacketFormat::BinaryFramed(spec));
        let out = d.push(&bytes);
        assert_eq!(out.packets.len(), 2, "the good frames must still decode");
        assert_eq!(out.packets[0].sequence, Some(1));
        assert_eq!(out.packets[1].sequence, Some(3));
        assert_eq!(d.checksum_failures, 1);
        assert!(out
            .rejections
            .iter()
            .any(|r| matches!(r, FrameRejection::ChecksumMismatch { .. })));
    }

    #[test]
    fn binary_version_mismatch_is_rejected() {
        let spec = default_spec();
        let mut frame = encode_frame(&spec, 1, 100, &[1.0, 2.0, 3.0]).unwrap();
        frame[spec.sync.len()] = 9;
        let mut d = PacketDecoder::new(PacketFormat::BinaryFramed(spec));
        let out = d.push(&frame);
        assert!(out.packets.is_empty());
        assert!(out
            .rejections
            .iter()
            .any(|r| matches!(r, FrameRejection::VersionMismatch { found: 9, .. })));
    }

    #[test]
    fn binary_payload_too_long_is_rejected_before_buffering_the_whole_frame() {
        let spec = BinaryFrameSpec {
            maximum_payload: 8,
            ..default_spec()
        };
        let mut frame = Vec::new();
        frame.extend_from_slice(&spec.sync);
        frame.push(spec.version);
        frame.extend_from_slice(&1u16.to_le_bytes()); // message type
        frame.extend_from_slice(&1u16.to_le_bytes()); // sequence
        frame.extend_from_slice(&0u32.to_le_bytes()); // timestamp
        frame.extend_from_slice(&1000u16.to_le_bytes()); // absurd payload length
        let mut d = PacketDecoder::new(PacketFormat::BinaryFramed(spec));
        let out = d.push(&frame);
        assert!(out.rejections.iter().any(|r| matches!(
            r,
            FrameRejection::PayloadTooLong {
                declared: 1000,
                maximum: 8
            }
        )));
        let text = out.rejections[0].detail();
        assert!(text.contains("1000"), "{}", text);
    }

    #[test]
    fn binary_sequence_gaps_are_counted() {
        let spec = default_spec();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&encode_frame(&spec, 1, 100, &[1.0, 1.0, 1.0]).unwrap());
        bytes.extend_from_slice(&encode_frame(&spec, 5, 200, &[2.0, 2.0, 2.0]).unwrap());
        let mut d = PacketDecoder::new(PacketFormat::BinaryFramed(spec));
        let out = d.push(&bytes);
        assert_eq!(out.packets.len(), 2);
        assert_eq!(d.sequence_gaps, 1);
    }

    #[test]
    fn binary_truncated_frame_waits_for_more_bytes() {
        let spec = default_spec();
        let frame = encode_frame(&spec, 1, 100, &[1.0, 2.0, 3.0]).unwrap();
        let mut d = PacketDecoder::new(PacketFormat::BinaryFramed(spec));
        let out = d.push(&frame[..frame.len() - 3]);
        assert!(out.packets.is_empty());
        assert!(out.rejections.is_empty());
        assert!(d.buffered_bytes() > 0);
        let out = d.push(&frame[frame.len() - 3..]);
        assert_eq!(out.packets.len(), 1);
    }

    #[test]
    fn field_encodings_decode_at_the_declared_width() {
        assert_eq!(FieldEncoding::U16.width(), 2);
        assert_eq!(FieldEncoding::I32.width(), 4);
        assert_eq!(FieldEncoding::F64.width(), 8);
        let bytes = 1234u16.to_le_bytes();
        assert!((FieldEncoding::U16.decode_le(&bytes).unwrap() - 1234.0).abs() < 1e-12);
        let bytes = (-5i16).to_le_bytes();
        assert!((FieldEncoding::I16.decode_le(&bytes).unwrap() + 5.0).abs() < 1e-12);
        let bytes = 1.5f32.to_be_bytes();
        assert!((FieldEncoding::F32.decode_be(&bytes).unwrap() - 1.5).abs() < 1e-6);
        assert!(FieldEncoding::F64.decode_le(&[0u8; 4]).is_none());
    }

    #[test]
    fn checksum_specs_compute_and_verify() {
        let body = b"hello frame";
        for spec in [
            ChecksumSpec::Crc32LittleEndian,
            ChecksumSpec::Crc32BigEndian,
            ChecksumSpec::Xor8,
        ] {
            let c = spec.compute(body);
            assert_eq!(c.len(), spec.width());
            assert!(spec.verify(body, &c));
            let mut tampered = c.clone();
            tampered[0] ^= 0xFF;
            assert!(!spec.verify(body, &tampered));
        }
        assert!(ChecksumSpec::None.compute(body).is_empty());
        assert!(ChecksumSpec::None.verify(body, &[]));
    }

    #[test]
    fn frame_spec_widths_are_consistent() {
        let spec = default_spec();
        // 2 sync + 1 version + 2 msgtype + 2 seq + 4 timestamp + 2 length = 13
        assert_eq!(spec.header_width(), 13);
        assert_eq!(spec.payload_width(), 12);
        assert_eq!(spec.frame_width(), 13 + 12 + 4);
        assert!(spec.validate().is_empty());
    }

    #[test]
    fn frame_spec_validation_catches_layout_mistakes() {
        let spec = BinaryFrameSpec {
            sync: [0xAA, 0xAA],
            fields: Vec::new(),
            ..default_spec()
        };
        let problems = spec.validate();
        assert!(problems.iter().any(|p| p.contains("sync")));
        assert!(problems.iter().any(|p| p.contains("payload fields")));

        let mut dup = default_spec();
        dup.fields[1].name = dup.fields[0].name.clone();
        assert!(dup.validate().iter().any(|p| p.contains("duplicate")));
    }

    #[test]
    fn field_interpret_applies_scale_and_offset() {
        let f = PayloadField {
            name: "ax".to_string(),
            encoding: FieldEncoding::I16,
            scale: 0.001,
            offset: 0.5,
        };
        assert!((f.interpret(1000.0) - 1.5).abs() < 1e-12);
    }

    #[test]
    fn format_metadata() {
        let csv = PacketFormat::default();
        assert_eq!(csv.label(), "CSV line");
        assert!(!csv.has_integrity_check());

        let bin = PacketFormat::default_binary(4);
        assert_eq!(bin.label(), "Binary framed");
        assert!(bin.has_integrity_check());
    }

    #[test]
    fn timebase_reflects_the_format() {
        let tb = timebase_for(&PacketFormat::default_binary(4));
        assert_eq!(tb.source, hex_core::TimeSource::Device);
        assert!(tb.note.as_ref().unwrap().contains("CRC32"));
        let tb = timebase_for(&PacketFormat::csv(vec![], TimestampUnit::Seconds));
        assert_eq!(tb.unit, TimestampUnit::Seconds);
    }

    #[test]
    fn packet_lookup_by_field_name() {
        let packet = DecodedPacket {
            device_time: Some(0.0),
            host_time: None,
            message_type: None,
            sequence: Some(1),
            values: vec![1.0, 2.0],
            field_names: vec!["ax".into(), "ay".into()],
            frame_bytes: 10,
            raw_payload: vec![],
        };
        assert_eq!(packet.value("ay"), Some(2.0));
        assert_eq!(packet.value("az"), None);
    }

    #[test]
    fn packets_to_columns_aligns_by_field_order() {
        let format = PacketFormat::csv(vec!["t".into(), "ax".into()], TimestampUnit::Seconds);
        let mut d = PacketDecoder::new(format);
        let out = d.push(b"0.0,1.0\n0.1,2.0\n");
        let (times, columns, names) = packets_to_columns(&out.packets);
        assert_eq!(times.len(), 2);
        assert_eq!(columns.len(), 2);
        assert_eq!(columns[1], vec![1.0, 2.0]);
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn decoder_reset_clears_partial_state() {
        let format = PacketFormat::csv(vec!["t".into(), "ax".into()], TimestampUnit::Seconds);
        let mut d = PacketDecoder::new(format);
        d.push(b"0.0,1.");
        assert!(d.buffered_bytes() > 0);
        d.reset();
        assert_eq!(d.buffered_bytes(), 0);
        assert_eq!(d.last_sequence(), None);
    }

    #[test]
    fn rejection_codes_are_stable() {
        assert_eq!(
            FrameRejection::LineTooLong {
                length: 1,
                maximum: 2
            }
            .code(),
            "packet.line_too_long"
        );
        assert_eq!(
            FrameRejection::NonFinite { field_index: 0 }.code(),
            "packet.non_finite"
        );
    }
}
