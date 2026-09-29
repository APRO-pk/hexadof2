//! The HexaDOF binary flight log: codec, format definition, and integrity rules.
//!
//! # Format definition, version 1
//!
//! The file extension is `.hlog`. Everything is **little endian**: fixed-width
//! integers are little-endian, and real values are IEEE-754 binary64. There is
//! no alignment padding anywhere; every field follows the previous one
//! immediately, so an offset is simply the sum of the preceding field widths.
//!
//! ## Log header (66 bytes)
//!
//! | Offset | Width | Field | Meaning |
//! |--------|-------|-------|---------|
//! | 0  | 8 | `magic`               | `b"HEXALOG1"` |
//! | 8  | 2 | `format_version`      | `1` for this revision |
//! | 10 | 2 | `flags`               | reserved, zero |
//! | 12 | 2 | `channel_count`       | number of channel descriptors |
//! | 14 | 8 | `nominal_dt_seconds`  | nominal sample interval, used to place samples inside a record |
//! | 22 | 8 | `created_unix_seconds`| signed Unix time at creation |
//! | 30 | 32| `device_id`           | NUL-padded UTF-8 device label |
//! | 62 | 4 | `channel_table_crc`   | CRC-32 of the channel table bytes |
//!
//! ## Channel table
//!
//! `channel_count` descriptors of 52 bytes each follow the header:
//!
//! | Offset | Width | Field |
//! |--------|-------|-------|
//! | 0  | 32 | `name`, NUL-padded UTF-8 |
//! | 32 | 1  | `role_code` |
//! | 33 | 1  | `unit_code` |
//! | 34 | 1  | `flags`, bit 0 set when the scale is the identity |
//! | 35 | 1  | `reserved`, zero |
//! | 36 | 8  | `scale_factor`, applied first |
//! | 44 | 8  | `scale_offset`, added after the factor |
//!
//! ## Data records
//!
//! Records are framed and each one is independently checksummed:
//!
//! | Offset | Width | Field |
//! |--------|-------|-------|
//! | 0 | 4 | `record_magic`, `0x52454348`, which reads as `b"HECR"` |
//! | 4 | 4 | `sequence`, starting at zero and incrementing by one |
//! | 8 | 2 | `sample_count` |
//! | 10| 2 | `payload_len`, bytes of payload, which must equal `sample_count * channel_count * 8` |
//! | 12| 8 | `timestamp_seconds`, the time of the first sample in the record |
//! | 20| `payload_len` | `sample_count * channel_count` little-endian binary64 values, sample-major |
//! | 20 + payload_len | 4 | `crc32` over every byte from `record_magic` through the last payload byte |
//!
//! Samples inside a record are placed on the time axis at
//! `timestamp_seconds + index * nominal_dt_seconds`.
//!
//! # Integrity rules
//!
//! A reader never interprets a byte that is not inside a validated frame. The
//! header magic, the format version, and the channel table checksum are checked
//! before any sample is read, and every record must match its own CRC before its
//! payload is converted. A record that fails its CRC is skipped, the reader
//! resynchronises on the next `record_magic`, and the failure is counted in
//! [`DecodeIntegrity::crc_failures`]. Only a bad magic, an unsupported version,
//! a bad channel table, or a self-inconsistent frame length abort the read.
//!
//! Because `payload_len` is 16 bits, a record holds at most
//! `65535 / (channel_count * 8)` samples; the writer clamps
//! `max_samples_per_record` accordingly.

use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use hex_core::Real;

use crate::channel::{Channel, ChannelRole};

/// File magic that opens every `.hlog` file.
pub const MAGIC: [u8; 8] = *b"HEXALOG1";

/// Format revision this crate writes and can read.
pub const CURRENT_FORMAT_VERSION: u16 = 1;

/// Record framing marker, which reads as `b"HECR"` in little-endian order.
pub const RECORD_MAGIC: [u8; 4] = *b"HECR";

/// Byte width of the log header.
pub const HEADER_LEN: usize = 66;

/// Byte width of one channel descriptor.
pub const DESCRIPTOR_LEN: usize = 52;

/// Byte width of a record's fixed fields, excluding payload and checksum.
pub const RECORD_PREFIX_LEN: usize = 20;

/// Unit code used when the declared unit has no compact code.
pub const UNIT_CODE_UNKNOWN: u8 = 255;

/// Default number of samples packed into one record.
pub const DEFAULT_MAX_SAMPLES_PER_RECORD: usize = 256;

/// Everything that can go wrong while encoding or decoding a binary log.
#[derive(Debug, Error)]
pub enum BinaryDecodeError {
    /// The file does not begin with [`MAGIC`].
    #[error("the file does not start with the HEXALOG1 magic")]
    BadMagic,
    /// The file uses a format revision this build cannot read.
    #[error("unsupported binary log version {found}; this build reads version {supported}")]
    UnsupportedVersion {
        /// Version found in the file.
        found: u16,
        /// Version this build supports.
        supported: u16,
    },
    /// The channel table does not match the checksum stored in the header.
    #[error("channel table checksum mismatch: header says {expected:#010x}, table computes {found:#010x}")]
    ChannelTableCrcMismatch {
        /// Checksum stored in the header.
        expected: u32,
        /// Checksum computed over the channel table bytes.
        found: u32,
    },
    /// A record failed its own checksum.
    #[error(
        "record {sequence} failed its checksum: stored {found:#010x}, computed {expected:#010x}"
    )]
    CrcMismatch {
        /// Sequence number of the damaged record.
        sequence: u32,
        /// Checksum computed over the record bytes.
        expected: u32,
        /// Checksum stored in the record.
        found: u32,
    },
    /// A frame declares a payload length that its own sample count contradicts.
    #[error(
        "The payload length does not match the declared frame length at byte offset {byte_offset}. \
         The stream may use the wrong baud rate or packet format."
    )]
    PayloadLengthMismatch {
        /// Payload length declared by the frame.
        declared: usize,
        /// Payload length implied by `sample_count * channel_count * 8`.
        available: usize,
        /// Byte offset of the start of the offending frame.
        byte_offset: usize,
    },
    /// The underlying reader or writer failed.
    #[error("binary log I/O failure: {source}")]
    Io {
        /// Operating-system error.
        #[from]
        source: std::io::Error,
    },
    /// The file ended before a complete structure could be read.
    #[error("the binary log ended in the middle of a structure")]
    Truncated,
}

impl BinaryDecodeError {
    /// Short message suitable for a banner in the import dialog.
    pub fn user_message(&self) -> String {
        match self {
            Self::BadMagic => {
                "This file is not a HexaDOF binary flight log, because its header magic is missing."
                    .to_string()
            }
            Self::UnsupportedVersion { found, supported } => format!(
                "This log uses format version {found}, but this build reads version {supported}."
            ),
            Self::ChannelTableCrcMismatch { .. } => {
                "The channel table in this log is damaged, so the column meanings cannot be trusted."
                    .to_string()
            }
            Self::CrcMismatch { sequence, .. } => {
                format!("Record {sequence} failed its checksum and was skipped.")
            }
            Self::PayloadLengthMismatch { byte_offset, .. } => format!(
                "The payload length does not match the declared frame length at byte offset {byte_offset}. \
                 The stream may use the wrong baud rate or packet format."
            ),
            Self::Io { source } => format!("The log could not be read: {source}"),
            Self::Truncated => "The log ends in the middle of a record.".to_string(),
        }
    }

    /// Concrete next step the user can take.
    pub fn suggested_action(&self) -> String {
        match self {
            Self::BadMagic => "Select the correct file, or convert it to CSV first.".to_string(),
            Self::UnsupportedVersion { .. } => {
                "Update HexaDOF, or export the log again with this version.".to_string()
            }
            Self::ChannelTableCrcMismatch { .. } => {
                "Re-download or re-export the log; the header cannot be repaired.".to_string()
            }
            Self::CrcMismatch { .. } => {
                "Check the link quality and re-record if the affected time range matters."
                    .to_string()
            }
            Self::PayloadLengthMismatch { .. } => {
                "Check the baud rate and the configured packet format for the serial link."
                    .to_string()
            }
            Self::Io { .. } => {
                "Check that the file is readable and not open elsewhere.".to_string()
            }
            Self::Truncated => {
                "The recording was interrupted; the readable records were still imported."
                    .to_string()
            }
        }
    }
}

/// Write-side log header, laid out exactly as the 66 on-disk bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogHeader {
    /// File magic, normally [`MAGIC`].
    pub magic: [u8; 8],
    /// Format revision.
    pub format_version: u16,
    /// Reserved flag bits, always zero in version 1.
    pub flags: u16,
    /// Number of channel descriptors. The writer sets this from the channel list.
    pub channel_count: u16,
    /// Nominal sample interval in seconds.
    pub nominal_dt_seconds: Real,
    /// Unix time at creation.
    pub created_unix_seconds: i64,
    /// Device label, NUL-padded UTF-8.
    pub device_id: [u8; 32],
    /// CRC-32 of the channel table. The writer recomputes this.
    pub channel_table_crc: u32,
}

impl Default for LogHeader {
    fn default() -> Self {
        Self {
            magic: MAGIC,
            format_version: CURRENT_FORMAT_VERSION,
            flags: 0,
            channel_count: 0,
            nominal_dt_seconds: 0.0,
            created_unix_seconds: 0,
            device_id: [0u8; 32],
            channel_table_crc: 0,
        }
    }
}

impl LogHeader {
    /// A header for a log with `nominal_dt_seconds` and no channels yet.
    pub fn new(nominal_dt_seconds: Real) -> Self {
        Self {
            nominal_dt_seconds,
            ..Self::default()
        }
    }

    /// Set the device label, truncating to the field width on a UTF-8 boundary.
    pub fn with_device_id(mut self, device_id: &str) -> Self {
        self.device_id = encode_fixed_array(device_id, 32);
        self
    }

    /// Set the creation time, in Unix seconds.
    pub fn with_created(mut self, unix_seconds: i64) -> Self {
        self.created_unix_seconds = unix_seconds;
        self
    }

    /// The device label with trailing padding removed.
    pub fn device_id_string(&self) -> String {
        decode_fixed_string(&self.device_id)
    }
}

/// Header as read back, with the device label already decoded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogHeaderDecoded {
    /// Format revision found in the file.
    pub format_version: u16,
    /// Reserved flag bits.
    pub flags: u16,
    /// Number of channel descriptors.
    pub channel_count: u16,
    /// Nominal sample interval in seconds.
    pub nominal_dt_seconds: Real,
    /// Unix time at creation.
    pub created_unix_seconds: i64,
    /// Device label.
    pub device_id: String,
    /// CRC-32 stored for the channel table.
    pub channel_table_crc: u32,
}

/// One channel entry in the log's channel table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelDescriptor {
    /// Column name, truncated to 32 bytes on a UTF-8 boundary.
    pub name: [u8; 32],
    /// Numeric role code, see [`ChannelRole::role_code`].
    pub role_code: u8,
    /// Compact unit code, see [`unit_code_for`].
    pub unit_code: u8,
    /// Flag bits, bit 0 set when the scale is the identity.
    pub flags: u8,
    /// Reserved, zero.
    pub reserved: u8,
    /// Multiplier applied to the stored value.
    pub scale_factor: Real,
    /// Offset added after the multiplier.
    pub scale_offset: Real,
}

impl Default for ChannelDescriptor {
    fn default() -> Self {
        Self {
            name: [0u8; 32],
            role_code: ChannelRole::Raw.role_code(),
            unit_code: UNIT_CODE_UNKNOWN,
            flags: 1,
            reserved: 0,
            scale_factor: 1.0,
            scale_offset: 0.0,
        }
    }
}

impl ChannelDescriptor {
    /// Build a descriptor from its parts, truncating the name if needed.
    pub fn new(name: &str, role: ChannelRole, scale_factor: Real, scale_offset: Real) -> Self {
        Self {
            name: encode_fixed_array(name, 32),
            role_code: role.role_code(),
            unit_code: unit_code_for(role.unit()),
            flags: u8::from(scale_factor == 1.0 && scale_offset == 0.0),
            reserved: 0,
            scale_factor,
            scale_offset,
        }
    }

    /// Build a descriptor that preserves a channel's declared unit and sign.
    ///
    /// The sign is folded into `scale_factor`, because the on-disk table has no
    /// separate sign field.
    pub fn from_channel(channel: &Channel) -> Self {
        let mut descriptor = Self::new(
            &channel.name,
            channel.role,
            channel.scale.factor * channel.sign,
            channel.scale.offset * channel.sign,
        );
        descriptor.unit_code = unit_code_for(&channel.unit);
        descriptor
    }

    /// The stored name with padding removed.
    pub fn name_string(&self) -> String {
        decode_fixed_string(&self.name)
    }

    /// The role this descriptor declares.
    pub fn role(&self) -> ChannelRole {
        ChannelRole::from_role_code(self.role_code)
    }

    /// True when the descriptor stores an identity scale.
    pub fn is_identity_scale(&self) -> bool {
        self.flags & 0x01 != 0
    }

    /// Convert a stored value into SI with this descriptor's scale.
    pub fn apply(&self, stored: Real) -> Real {
        stored * self.scale_factor + self.scale_offset
    }
}

/// Channel descriptor as read back, with the name and unit already decoded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelDescriptorDecoded {
    /// Column name.
    pub name: String,
    /// Physical role.
    pub role: ChannelRole,
    /// Unit label implied by the unit code.
    pub unit: String,
    /// Raw flag bits.
    pub flags: u8,
    /// Multiplier stored in the descriptor.
    pub scale_factor: Real,
    /// Offset stored in the descriptor.
    pub scale_offset: Real,
}

impl ChannelDescriptorDecoded {
    /// Convert a stored value into SI with this descriptor's scale.
    pub fn apply(&self, stored: Real) -> Real {
        stored * self.scale_factor + self.scale_offset
    }

    /// Rebuild a normalised channel from this descriptor and its samples.
    ///
    /// The stored scale becomes the channel's [`hex_core::UnitScale`], so
    /// `corrected_values` reproduces exactly what `apply` would return.
    pub fn to_channel(&self, values: Vec<Real>) -> Channel {
        let mut channel = Channel::new(self.name.clone(), self.role, values);
        channel.unit = self.unit.clone();
        channel.scale = hex_core::UnitScale::new(self.scale_factor, self.scale_offset);
        channel.unit_confidence = hex_core::units::UnitConfidence::Declared;
        channel
    }

    /// Recover the compact unit code implied by this descriptor's unit label.
    pub fn unit_code_round_trip(&self) -> u8 {
        unit_code_for(&self.unit)
    }
}

/// Running integrity counters for one decode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecodeIntegrity {
    /// Records that failed their checksum and were skipped.
    pub crc_failures: usize,
    /// Records cut short by the end of the stream.
    pub truncated_records: usize,
    /// Times the sequence number skipped forward.
    pub sequence_gaps: usize,
    /// Records successfully decoded.
    pub records_read: usize,
    /// Samples successfully decoded.
    pub samples_read: usize,
    /// Bytes consumed from the stream.
    pub bytes_consumed: usize,
}

impl DecodeIntegrity {
    /// True when every record decoded and the framing was continuous.
    pub fn is_clean(&self) -> bool {
        self.crc_failures == 0 && self.truncated_records == 0 && self.sequence_gaps == 0
    }

    /// One-line summary for the import report.
    pub fn summary(&self) -> String {
        format!(
            "{} record(s), {} sample(s); {} checksum failure(s), {} truncated record(s), {} sequence gap(s)",
            self.records_read,
            self.samples_read,
            self.crc_failures,
            self.truncated_records,
            self.sequence_gaps
        )
    }
}

/// A fully decoded binary log.
#[derive(Debug, Clone, PartialEq)]
pub struct BinaryLog {
    /// Header as read.
    pub header: LogHeaderDecoded,
    /// Channel table as read.
    pub channels: Vec<ChannelDescriptorDecoded>,
    /// One timestamp per sample, reconstructed from record times and the nominal interval.
    pub times: Vec<Real>,
    /// One numeric column per channel, all the same length as `times`.
    pub columns: Vec<Vec<Real>>,
    /// Integrity counters for the whole decode.
    pub integrity: DecodeIntegrity,
}

impl BinaryLog {
    /// Number of samples decoded.
    pub fn sample_count(&self) -> usize {
        self.times.len()
    }

    /// Index of a channel by name, case-insensitively.
    pub fn channel_index(&self, name: &str) -> Option<usize> {
        self.channels
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(name.trim()))
    }
}

/// One decoded record delivered by [`LogStreamReader`].
#[derive(Debug, Clone, PartialEq)]
pub struct StreamRecord {
    /// Sequence number stored in the frame.
    pub sequence: u32,
    /// Time of the first sample in the record.
    pub timestamp_seconds: Real,
    /// Number of samples in the record.
    pub sample_count: usize,
    /// Sample-major payload, `sample_count * channel_count` values.
    pub values: Vec<Real>,
    /// Byte offset of the record's first byte.
    pub byte_offset: usize,
}

/// Encode a string into a fixed 32-byte NUL-padded field, never splitting a
/// multi-byte character.
pub fn encode_fixed_array(value: &str, width: usize) -> [u8; 32] {
    debug_assert_eq!(width, 32);
    let mut out = [0u8; 32];
    let bytes = value.as_bytes();
    let mut take = bytes.len().min(32);
    while take > 0 && !value.is_char_boundary(take) {
        take -= 1;
    }
    out[..take].copy_from_slice(&bytes[..take]);
    out
}

/// Decode a NUL-padded field back into a string, dropping invalid bytes.
pub fn decode_fixed_string(field: &[u8]) -> String {
    let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

/// Compact unit code stored in a channel descriptor.
///
/// The code is a hint for readers that do not know the channel by name. The
/// authoritative conversion is always `scale_factor` and `scale_offset`.
pub fn unit_code_for(unit: &str) -> u8 {
    match unit.trim() {
        "s" => 1,
        "m/s^2" => 2,
        "rad/s" => 3,
        "T" => 4,
        "Pa" => 5,
        "K" => 6,
        "deg" => 7,
        "m" => 8,
        "m/s" => 9,
        "V" => 10,
        "A" => 11,
        "1" => 12,
        "rad" => 13,
        "C" => 14,
        _ => UNIT_CODE_UNKNOWN,
    }
}

/// Unit label implied by a compact unit code.
pub fn unit_for_code(code: u8) -> String {
    match code {
        1 => "s",
        2 => "m/s^2",
        3 => "rad/s",
        4 => "T",
        5 => "Pa",
        6 => "K",
        7 => "deg",
        8 => "m",
        9 => "m/s",
        10 => "V",
        11 => "A",
        12 => "1",
        13 => "rad",
        14 => "C",
        _ => "",
    }
    .to_string()
}

fn write_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_i64(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_f64(out: &mut Vec<u8>, value: Real) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_f64(bytes: &[u8], offset: usize) -> Real {
    let mut raw = [0u8; 8];
    raw.copy_from_slice(&bytes[offset..offset + 8]);
    Real::from_le_bytes(raw)
}

fn encode_descriptor(out: &mut Vec<u8>, descriptor: &ChannelDescriptor) {
    out.extend_from_slice(&descriptor.name);
    out.push(descriptor.role_code);
    out.push(descriptor.unit_code);
    out.push(descriptor.flags);
    out.push(descriptor.reserved);
    write_f64(out, descriptor.scale_factor);
    write_f64(out, descriptor.scale_offset);
}

fn encode_channel_table(channels: &[ChannelDescriptor]) -> Vec<u8> {
    let mut out = Vec::with_capacity(channels.len() * DESCRIPTOR_LEN);
    for descriptor in channels {
        encode_descriptor(&mut out, descriptor);
    }
    out
}

/// Build the 66-byte header plus the channel table.
fn encode_header_block(header: &LogHeader, channels: &[ChannelDescriptor]) -> (Vec<u8>, LogHeader) {
    let table = encode_channel_table(channels);
    let mut resolved = header.clone();
    resolved.magic = MAGIC;
    resolved.format_version = CURRENT_FORMAT_VERSION;
    resolved.channel_count = channels.len().min(u16::MAX as usize) as u16;
    resolved.channel_table_crc = crc32fast::hash(&table);

    let mut out = Vec::with_capacity(HEADER_LEN + table.len());
    out.extend_from_slice(&resolved.magic);
    write_u16(&mut out, resolved.format_version);
    write_u16(&mut out, resolved.flags);
    write_u16(&mut out, resolved.channel_count);
    write_f64(&mut out, resolved.nominal_dt_seconds);
    write_i64(&mut out, resolved.created_unix_seconds);
    out.extend_from_slice(&resolved.device_id);
    write_u32(&mut out, resolved.channel_table_crc);
    out.extend_from_slice(&table);
    (out, resolved)
}

fn decode_header(bytes: &[u8]) -> Result<LogHeaderDecoded, BinaryDecodeError> {
    if bytes.len() < HEADER_LEN {
        return Err(BinaryDecodeError::Truncated);
    }
    if bytes[..8] != MAGIC {
        return Err(BinaryDecodeError::BadMagic);
    }
    let format_version = read_u16(bytes, 8);
    if format_version != CURRENT_FORMAT_VERSION {
        return Err(BinaryDecodeError::UnsupportedVersion {
            found: format_version,
            supported: CURRENT_FORMAT_VERSION,
        });
    }
    Ok(LogHeaderDecoded {
        format_version,
        flags: read_u16(bytes, 10),
        channel_count: read_u16(bytes, 12),
        nominal_dt_seconds: read_f64(bytes, 14),
        created_unix_seconds: i64::from_le_bytes(
            bytes[22..30]
                .try_into()
                .map_err(|_| BinaryDecodeError::Truncated)?,
        ),
        device_id: decode_fixed_string(&bytes[30..62]),
        channel_table_crc: read_u32(bytes, 62),
    })
}

fn decode_channel_table(
    header: &LogHeaderDecoded,
    rest: &[u8],
) -> Result<Vec<ChannelDescriptorDecoded>, BinaryDecodeError> {
    let count = header.channel_count as usize;
    let table_len = count * DESCRIPTOR_LEN;
    if rest.len() < table_len {
        return Err(BinaryDecodeError::Truncated);
    }
    let table = &rest[..table_len];
    let found = crc32fast::hash(table);
    if found != header.channel_table_crc {
        return Err(BinaryDecodeError::ChannelTableCrcMismatch {
            expected: header.channel_table_crc,
            found,
        });
    }

    let mut channels = Vec::with_capacity(count);
    for index in 0..count {
        let base = index * DESCRIPTOR_LEN;
        let entry = &table[base..base + DESCRIPTOR_LEN];
        channels.push(ChannelDescriptorDecoded {
            name: decode_fixed_string(&entry[..32]),
            role: ChannelRole::from_role_code(entry[32]),
            unit: unit_for_code(entry[33]),
            flags: entry[34],
            scale_factor: read_f64(entry, 36),
            scale_offset: read_f64(entry, 44),
        });
    }
    Ok(channels)
}

/// Streams framed records out of a reader, resynchronising after a bad checksum.
///
/// The reader never returns payload bytes that were not covered by a verified
/// frame checksum, so a partial or garbled record can never corrupt the columns.
pub struct LogStreamReader<R: Read> {
    inner: R,
    buf: Vec<u8>,
    base_offset: usize,
    eof: bool,
    finished: bool,
    header: Option<LogHeaderDecoded>,
    channels: Vec<ChannelDescriptorDecoded>,
    integrity: DecodeIntegrity,
    last_sequence: Option<u32>,
}

impl<R: Read> LogStreamReader<R> {
    /// Wrap a reader. Nothing is consumed until the first record is requested.
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            buf: Vec::new(),
            base_offset: 0,
            eof: false,
            finished: false,
            header: None,
            channels: Vec::new(),
            integrity: DecodeIntegrity::default(),
            last_sequence: None,
        }
    }

    /// Read and verify the header and the channel table.
    ///
    /// Calling this is optional; [`Self::next_record`] does it on demand.
    pub fn prepare(&mut self) -> Result<(), BinaryDecodeError> {
        if self.header.is_some() {
            return Ok(());
        }
        self.fill_to(HEADER_LEN)?;
        if self.buf.len() < HEADER_LEN {
            return Err(BinaryDecodeError::Truncated);
        }
        let header = decode_header(&self.buf)?;
        let table_len = header.channel_count as usize * DESCRIPTOR_LEN;
        self.fill_to(HEADER_LEN + table_len)?;
        if self.buf.len() < HEADER_LEN + table_len {
            return Err(BinaryDecodeError::Truncated);
        }
        let channels = decode_channel_table(&header, &self.buf[HEADER_LEN..])?;
        let consumed = HEADER_LEN + table_len;
        self.consume(consumed);
        self.channels = channels;
        self.header = Some(header);
        Ok(())
    }

    /// The decoded header, once [`Self::prepare`] has run.
    pub fn header(&self) -> Option<&LogHeaderDecoded> {
        self.header.as_ref()
    }

    /// The decoded channel table, once [`Self::prepare`] has run.
    pub fn channels(&self) -> &[ChannelDescriptorDecoded] {
        &self.channels
    }

    /// Integrity counters accumulated so far.
    pub fn integrity(&self) -> DecodeIntegrity {
        self.integrity
    }

    /// Decode the next record, or `None` at the end of the stream.
    ///
    /// A record with a bad checksum is counted and skipped, and the reader
    /// resynchronises on the next framing marker.
    pub fn next_record(&mut self) -> Result<Option<StreamRecord>, BinaryDecodeError> {
        if self.finished {
            return Ok(None);
        }
        self.prepare()?;
        let channel_count = self.channels.len();

        loop {
            self.fill_to(4)?;
            if self.buf.is_empty() {
                self.finished = true;
                self.integrity.bytes_consumed = self.base_offset;
                return Ok(None);
            }
            if self.buf.len() < 4 {
                // A trailing fragment too short to be a frame.
                self.integrity.truncated_records += 1;
                self.consume(self.buf.len());
                self.finished = true;
                return Ok(None);
            }
            if self.buf[..4] != RECORD_MAGIC[..] {
                match find_record_magic(&self.buf[1..]) {
                    Some(index) => {
                        self.consume(index + 1);
                        continue;
                    }
                    None => {
                        self.consume(self.buf.len());
                        self.finished = true;
                        self.integrity.bytes_consumed = self.base_offset;
                        return Ok(None);
                    }
                }
            }

            let record_offset = self.base_offset;
            self.fill_to(RECORD_PREFIX_LEN)?;
            if self.buf.len() < RECORD_PREFIX_LEN {
                self.integrity.truncated_records += 1;
                self.consume(self.buf.len());
                self.finished = true;
                self.integrity.bytes_consumed = self.base_offset;
                return Ok(None);
            }

            let sequence = read_u32(&self.buf, 4);
            let sample_count = read_u16(&self.buf, 8) as usize;
            let payload_len = read_u16(&self.buf, 10) as usize;
            let timestamp_seconds = read_f64(&self.buf, 12);

            let implied = sample_count * channel_count * 8;
            if payload_len != implied {
                return Err(BinaryDecodeError::PayloadLengthMismatch {
                    declared: payload_len,
                    available: implied,
                    byte_offset: record_offset,
                });
            }

            let frame_len = RECORD_PREFIX_LEN + payload_len + 4;
            self.fill_to(frame_len)?;
            if self.buf.len() < frame_len {
                self.integrity.truncated_records += 1;
                self.consume(self.buf.len());
                self.finished = true;
                self.integrity.bytes_consumed = self.base_offset;
                return Ok(None);
            }

            let stored_crc = read_u32(&self.buf, frame_len - 4);
            let computed_crc = crc32fast::hash(&self.buf[..frame_len - 4]);
            if stored_crc != computed_crc {
                self.integrity.crc_failures += 1;
                // Step past the frame marker and resynchronise on the next one.
                self.consume(4);
                continue;
            }

            let mut values = Vec::with_capacity(sample_count * channel_count);
            for index in 0..sample_count * channel_count {
                values.push(read_f64(&self.buf, RECORD_PREFIX_LEN + index * 8));
            }

            if let Some(previous) = self.last_sequence {
                if sequence != previous.wrapping_add(1) {
                    self.integrity.sequence_gaps += 1;
                }
            }
            self.last_sequence = Some(sequence);
            self.integrity.records_read += 1;
            self.integrity.samples_read += sample_count;

            self.consume(frame_len);
            self.integrity.bytes_consumed = self.base_offset;

            return Ok(Some(StreamRecord {
                sequence,
                timestamp_seconds,
                sample_count,
                values,
                byte_offset: record_offset,
            }));
        }
    }

    fn fill_to(&mut self, wanted: usize) -> Result<(), BinaryDecodeError> {
        let mut chunk = [0u8; 8192];
        while self.buf.len() < wanted && !self.eof {
            let read = self.inner.read(&mut chunk)?;
            if read == 0 {
                self.eof = true;
            } else {
                self.buf.extend_from_slice(&chunk[..read]);
            }
        }
        Ok(())
    }

    fn consume(&mut self, count: usize) {
        let count = count.min(self.buf.len());
        self.buf.drain(..count);
        self.base_offset += count;
    }
}

fn find_record_magic(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(RECORD_MAGIC.len())
        .position(|w| w == RECORD_MAGIC.as_slice())
}

/// Builds a `.hlog` byte stream.
///
/// Values are stored exactly as supplied; no rounding or unit conversion
/// happens here, so a round trip is bit-exact for finite input.
pub struct BinaryWriter {
    header: LogHeader,
    channels: Vec<ChannelDescriptor>,
    out: Vec<u8>,
    pending_times: Vec<Real>,
    pending_values: Vec<Real>,
    sequence: u32,
    effective_samples_per_record: usize,
    /// Requested cap on samples per record. Clamped to what a 16-bit payload
    /// length field can describe.
    pub max_samples_per_record: usize,
    /// A sample whose time departs from the expected time by more than this
    /// starts a new record, so a gap between samples survives the round trip.
    /// Zero disables gap splitting, which is only correct for a log whose header
    /// declares no nominal interval.
    pub gap_tolerance_seconds: Real,
}

impl BinaryWriter {
    /// Start a log. The header's `channel_count` and `channel_table_crc` are
    /// recomputed from `channels` and override whatever the caller supplied.
    pub fn new(header: LogHeader, channels: Vec<ChannelDescriptor>) -> Self {
        let (block, resolved) = encode_header_block(&header, &channels);
        let channel_count = channels.len();
        let per_record_bytes = channel_count.max(1) * 8;
        let effective_samples_per_record =
            (u16::MAX as usize / per_record_bytes).clamp(1, u16::MAX as usize);
        let gap_tolerance_seconds = if resolved.nominal_dt_seconds > 0.0 {
            resolved.nominal_dt_seconds * 0.5
        } else {
            0.0
        };
        Self {
            header: resolved,
            channels,
            out: block,
            pending_times: Vec::new(),
            pending_values: Vec::new(),
            sequence: 0,
            effective_samples_per_record,
            max_samples_per_record: DEFAULT_MAX_SAMPLES_PER_RECORD,
            gap_tolerance_seconds,
        }
    }

    /// The resolved header, including the computed channel table checksum.
    pub fn header(&self) -> &LogHeader {
        &self.header
    }

    /// The channel table this writer will emit.
    pub fn channels(&self) -> &[ChannelDescriptor] {
        &self.channels
    }

    /// Samples still buffered in the open record.
    pub fn buffered_samples(&self) -> usize {
        self.pending_times.len()
    }

    /// Append one sample.
    ///
    /// `values` must hold exactly one value per channel, in channel-table order.
    /// The record is flushed automatically once the per-record cap is reached.
    ///
    /// A sample that arrives late, or early, by more than
    /// [`Self::gap_tolerance_seconds`] starts a new record. Inside a record the
    /// reader places samples on the nominal interval, so without that split a
    /// dropped packet would be smeared into uniform samples and the gap would
    /// disappear from the data.
    pub fn push_sample(
        &mut self,
        time_seconds: Real,
        values: &[Real],
    ) -> Result<(), BinaryDecodeError> {
        if values.len() != self.channels.len() {
            return Err(BinaryDecodeError::PayloadLengthMismatch {
                declared: values.len() * 8,
                available: self.channels.len() * 8,
                byte_offset: self.out.len(),
            });
        }
        if self.gap_tolerance_seconds > 0.0 && !self.pending_times.is_empty() {
            // The reader places the pending samples on the nominal grid anchored
            // at the first one, so this is where the new sample would land.
            let count = self.pending_times.len();
            let grid_position =
                self.pending_times[0] + count as Real * self.header.nominal_dt_seconds;
            // A sample that does not belong on the grid opens a new record. That
            // preserves a real gap instead of smearing it, re-anchors a clock
            // that is running at a different rate, and keeps the decoded time
            // axis strictly increasing, because the tolerance is half the
            // nominal interval and the split therefore lands past the previous
            // record's last reconstructed sample.
            if (time_seconds - grid_position).abs() > self.gap_tolerance_seconds {
                self.flush_record();
            }
        }
        self.pending_times.push(time_seconds);
        self.pending_values.extend_from_slice(values);
        if self.pending_times.len() >= self.record_cap() {
            self.flush_record();
        }
        Ok(())
    }

    fn record_cap(&self) -> usize {
        self.max_samples_per_record
            .max(1)
            .min(self.effective_samples_per_record)
    }

    fn flush_record(&mut self) {
        let sample_count = self.pending_times.len();
        if sample_count == 0 {
            return;
        }
        let payload_len = sample_count * self.channels.len() * 8;
        let mut frame = Vec::with_capacity(RECORD_PREFIX_LEN + payload_len + 4);
        frame.extend_from_slice(&RECORD_MAGIC);
        write_u32(&mut frame, self.sequence);
        write_u16(&mut frame, sample_count as u16);
        write_u16(&mut frame, payload_len as u16);
        write_f64(&mut frame, self.pending_times[0]);
        for value in &self.pending_values {
            write_f64(&mut frame, *value);
        }
        let crc = crc32fast::hash(&frame);
        write_u32(&mut frame, crc);
        self.out.extend_from_slice(&frame);

        self.sequence = self.sequence.wrapping_add(1);
        self.pending_times.clear();
        self.pending_values.clear();
    }

    /// Flush the open record and return the complete file contents.
    pub fn finish(mut self) -> Result<Vec<u8>, BinaryDecodeError> {
        self.flush_record();
        Ok(self.out)
    }

    /// Flush the open record and write the file, returning the byte count.
    pub fn write_to_file<P: AsRef<Path>>(self, path: P) -> Result<usize, BinaryDecodeError> {
        let bytes = self.finish()?;
        std::fs::write(path, &bytes)?;
        Ok(bytes.len())
    }
}

/// Stateless decode helpers.
pub struct BinaryReader;

impl BinaryReader {
    /// Decode a complete log from memory.
    ///
    /// Damaged records are skipped and counted; a bad magic, an unsupported
    /// version, a bad channel table, or an inconsistent frame length is fatal.
    pub fn read_all(bytes: &[u8]) -> Result<BinaryLog, BinaryDecodeError> {
        let mut reader = LogStreamReader::new(bytes);
        reader.prepare()?;
        let header = reader
            .header()
            .cloned()
            .ok_or(BinaryDecodeError::Truncated)?;
        let channels = reader.channels().to_vec();
        let channel_count = channels.len();
        let nominal_dt = header.nominal_dt_seconds;
        let mut times = Vec::new();
        let mut columns: Vec<Vec<Real>> = vec![Vec::new(); channel_count];

        while let Some(record) = reader.next_record()? {
            for sample in 0..record.sample_count {
                let offset = sample * channel_count;
                times.push(record.timestamp_seconds + sample as Real * nominal_dt);
                for (index, column) in columns.iter_mut().enumerate() {
                    column.push(record.values[offset + index]);
                }
            }
        }

        Ok(BinaryLog {
            header,
            channels,
            times,
            columns,
            integrity: reader.integrity(),
        })
    }

    /// Decode a complete log from disk.
    pub fn read_file<P: AsRef<Path>>(path: P) -> Result<BinaryLog, BinaryDecodeError> {
        let bytes = std::fs::read(path)?;
        Self::read_all(&bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptors(count: usize) -> Vec<ChannelDescriptor> {
        let roles = [
            ChannelRole::Time,
            ChannelRole::AccelX,
            ChannelRole::AccelY,
            ChannelRole::AccelZ,
            ChannelRole::GyroX,
            ChannelRole::GyroY,
            ChannelRole::GyroZ,
        ];
        (0..count)
            .map(|i| {
                let role = roles[i % roles.len()];
                ChannelDescriptor::new(&format!("ch{i}"), role, 1.0, 0.0)
            })
            .collect()
    }

    fn write_log(samples: usize, channels: usize, per_record: usize) -> Vec<u8> {
        let header = LogHeader::new(0.01)
            .with_device_id("TESTDEV")
            .with_created(1_700_000_000);
        let mut writer = BinaryWriter::new(header, descriptors(channels));
        writer.max_samples_per_record = per_record;
        for i in 0..samples {
            let values: Vec<Real> = (0..channels).map(|c| (i * 10 + c) as Real * 0.5).collect();
            writer.push_sample(i as Real * 0.01, &values).unwrap();
        }
        writer.finish().unwrap()
    }

    #[test]
    fn writer_and_reader_round_trip_exactly() {
        let bytes = write_log(10, 3, 256);
        let log = BinaryReader::read_all(&bytes).unwrap();
        assert_eq!(log.channels.len(), 3);
        assert_eq!(log.sample_count(), 10);
        assert_eq!(log.columns.len(), 3);
        assert_eq!(log.header.device_id, "TESTDEV");
        assert!(log.integrity.is_clean());
        assert_eq!(log.integrity.records_read, 1);
        assert_eq!(log.integrity.samples_read, 10);
        for i in 0..10 {
            assert_eq!(log.times[i], i as Real * 0.01);
            for c in 0..3 {
                assert_eq!(log.columns[c][i], (i * 10 + c) as Real * 0.5);
            }
        }
    }

    #[test]
    fn header_layout_is_66_bytes_and_table_follows() {
        let bytes = write_log(1, 4, 256);
        assert_eq!(&bytes[..8], b"HEXALOG1");
        assert_eq!(read_u16(&bytes, 8), CURRENT_FORMAT_VERSION);
        assert_eq!(read_u16(&bytes, 12), 4);
        assert_eq!(
            bytes.len(),
            HEADER_LEN + 4 * DESCRIPTOR_LEN + RECORD_PREFIX_LEN + 4 * 8 + 4
        );
    }

    #[test]
    fn record_framing_matches_the_documented_layout() {
        let bytes = write_log(3, 2, 256);
        let record_start = HEADER_LEN + 2 * DESCRIPTOR_LEN;
        assert_eq!(&bytes[record_start..record_start + 4], b"HECR");
        assert_eq!(read_u32(&bytes, record_start + 4), 0);
        assert_eq!(read_u16(&bytes, record_start + 8), 3);
        assert_eq!(read_u16(&bytes, record_start + 10), 3 * 2 * 8);
        assert_eq!(read_f64(&bytes, record_start + 12), 0.0);
    }

    #[test]
    fn bad_magic_is_fatal() {
        let mut bytes = write_log(2, 2, 256);
        bytes[0] = b'X';
        let err = BinaryReader::read_all(&bytes).unwrap_err();
        assert!(matches!(err, BinaryDecodeError::BadMagic));
        assert!(err
            .user_message()
            .contains("not a HexaDOF binary flight log"));
    }

    #[test]
    fn unsupported_version_is_fatal() {
        let mut bytes = write_log(2, 2, 256);
        bytes[8] = 99;
        let err = BinaryReader::read_all(&bytes).unwrap_err();
        match err {
            BinaryDecodeError::UnsupportedVersion { found, supported } => {
                assert_eq!(found, 99);
                assert_eq!(supported, CURRENT_FORMAT_VERSION);
            }
            other => panic!("expected version error, got {other:?}"),
        }
    }

    #[test]
    fn channel_table_crc_mismatch_is_fatal() {
        let mut bytes = write_log(2, 2, 256);
        // Corrupt one byte inside the first descriptor's name field.
        bytes[HEADER_LEN + 1] = b'Z';
        let err = BinaryReader::read_all(&bytes).unwrap_err();
        match err {
            BinaryDecodeError::ChannelTableCrcMismatch { expected, found } => {
                assert_ne!(expected, found);
            }
            other => panic!("expected channel table error, got {other:?}"),
        }
    }

    #[test]
    fn record_crc_mismatch_resynchronises_and_counts() {
        let mut bytes = write_log(6, 2, 2);
        let second_record = HEADER_LEN + 2 * DESCRIPTOR_LEN + (RECORD_PREFIX_LEN + 2 * 2 * 8 + 4);
        // Flip a byte inside the second record's payload.
        bytes[second_record + RECORD_PREFIX_LEN] ^= 0xFF;

        let log = BinaryReader::read_all(&bytes).unwrap();
        assert_eq!(log.integrity.crc_failures, 1);
        assert_eq!(log.integrity.records_read, 2);
        assert_eq!(log.integrity.samples_read, 4);
        assert_eq!(log.sample_count(), 4);
        // The surviving records are the first and the third.
        assert_eq!(log.times[0], 0.0);
        assert_eq!(log.times[2], 0.04);
    }

    #[test]
    fn crc_mismatch_in_the_last_record_keeps_the_earlier_ones() {
        let bytes = write_log(4, 2, 2);
        let last_record = HEADER_LEN + 2 * DESCRIPTOR_LEN + (RECORD_PREFIX_LEN + 2 * 2 * 8 + 4);
        let mut damaged = bytes.clone();
        damaged[last_record + RECORD_PREFIX_LEN + 1] ^= 0x01;
        let log = BinaryReader::read_all(&damaged).unwrap();
        assert_eq!(log.integrity.crc_failures, 1);
        assert_eq!(log.sample_count(), 2);
    }

    #[test]
    fn truncated_final_record_is_counted_and_not_fatal() {
        let mut bytes = write_log(4, 2, 2);
        // Append the first 10 bytes of what would be another record.
        bytes.extend_from_slice(&RECORD_MAGIC);
        bytes.extend_from_slice(&7u32.to_le_bytes());
        bytes.extend_from_slice(&[0u8, 0u8]);
        let log = BinaryReader::read_all(&bytes).unwrap();
        assert_eq!(log.integrity.truncated_records, 1);
        assert_eq!(log.integrity.crc_failures, 0);
        assert_eq!(log.sample_count(), 4);
        assert!(!log.integrity.is_clean());
    }

    #[test]
    fn truncated_header_is_fatal() {
        let bytes = write_log(4, 2, 2);
        let err = BinaryReader::read_all(&bytes[..20]).unwrap_err();
        assert!(matches!(err, BinaryDecodeError::Truncated));
    }

    #[test]
    fn missing_sequence_number_counts_as_a_gap() {
        let bytes = write_log(6, 2, 2);
        let record_len = RECORD_PREFIX_LEN + 2 * 2 * 8 + 4;
        let first = HEADER_LEN + 2 * DESCRIPTOR_LEN;
        let mut damaged = Vec::new();
        damaged.extend_from_slice(&bytes[..first + record_len]);
        damaged.extend_from_slice(&bytes[first + 2 * record_len..]);

        let log = BinaryReader::read_all(&damaged).unwrap();
        assert_eq!(log.integrity.sequence_gaps, 1);
        assert_eq!(log.integrity.records_read, 2);
        assert_eq!(log.sample_count(), 4);
    }

    #[test]
    fn payload_length_mismatch_reports_the_offset_and_the_message() {
        let mut bytes = write_log(4, 2, 2);
        let record_start = HEADER_LEN + 2 * DESCRIPTOR_LEN;
        // Claim a payload that sample_count * channel_count * 8 does not support.
        bytes[record_start + 10] = 0xFF;
        bytes[record_start + 11] = 0x00;

        let err = BinaryReader::read_all(&bytes).unwrap_err();
        match err {
            BinaryDecodeError::PayloadLengthMismatch {
                declared,
                available,
                byte_offset,
            } => {
                assert_eq!(declared, 255);
                assert_eq!(available, 2 * 2 * 8);
                assert_eq!(byte_offset, record_start);
            }
            other => panic!("expected payload length error, got {other:?}"),
        }

        let bytes = write_log(4, 2, 2);
        let mut damaged = bytes.clone();
        damaged[record_start + 10] = 0xFF;
        let message = BinaryReader::read_all(&damaged).unwrap_err().to_string();
        assert!(message.contains("payload length does not match"));
        assert!(message.contains(&format!("byte offset {record_start}")));
        assert!(message.contains("baud rate"));
    }

    #[test]
    fn large_log_spans_many_records_and_preserves_every_sample() {
        let samples = 1000;
        let channels = 6;
        let bytes = write_log(samples, channels, 64);
        let log = BinaryReader::read_all(&bytes).unwrap();
        assert_eq!(log.sample_count(), samples);
        assert_eq!(
            log.integrity.records_read,
            (samples as f64 / 64.0).ceil() as usize
        );
        assert!(log.integrity.is_clean());
        for i in 0..samples {
            for c in 0..channels {
                assert_eq!(log.columns[c][i], (i * 10 + c) as Real * 0.5);
            }
        }
        assert!((log.times[999] - 9.99).abs() < 1e-9);
    }

    #[test]
    fn sample_times_inside_a_record_use_the_nominal_interval() {
        let bytes = write_log(5, 1, 256);
        let log = BinaryReader::read_all(&bytes).unwrap();
        for i in 0..5 {
            assert!((log.times[i] - i as Real * 0.01).abs() < 1e-12);
        }
    }

    #[test]
    fn a_gap_between_samples_survives_the_round_trip() {
        // A dropped packet must not be smeared into uniform samples. The writer
        // starts a new record at the discontinuity, so the missing time is still
        // missing when the log is read back.
        let mut writer = BinaryWriter::new(LogHeader::new(0.01), descriptors(1));
        for i in 0..10 {
            writer.push_sample(i as Real * 0.01, &[i as Real]).unwrap();
        }
        // A half second stall, then samples resume.
        for i in 0..10 {
            writer
                .push_sample(0.59 + i as Real * 0.01, &[(100 + i) as Real])
                .unwrap();
        }
        let bytes = writer.finish().unwrap();
        let log = BinaryReader::read_all(&bytes).unwrap();
        assert_eq!(log.sample_count(), 20);
        assert!(log.integrity.is_clean());
        assert!((log.times[9] - 0.09).abs() < 1e-12);
        assert!(
            (log.times[10] - 0.59).abs() < 1e-12,
            "the gap must be preserved, not closed: {}",
            log.times[10]
        );
        // The interval across the gap is the real one, not the nominal one.
        let across = log.times[10] - log.times[9];
        assert!((across - 0.5).abs() < 1e-12, "gap was {}", across);
        assert!(log.times.windows(2).all(|w| w[1] > w[0]));
    }

    #[test]
    fn a_clock_running_at_a_different_rate_still_decodes_in_order() {
        // The header's nominal interval is computed over the whole session. A
        // stretch that runs faster than that average must not let the nominal
        // grid run ahead of the real samples until the time axis folds back.
        let mut writer = BinaryWriter::new(LogHeader::new(0.011), descriptors(1));
        for i in 0..500 {
            writer.push_sample(i as Real * 0.01, &[i as Real]).unwrap();
        }
        let bytes = writer.finish().unwrap();
        let log = BinaryReader::read_all(&bytes).unwrap();
        assert_eq!(log.sample_count(), 500);
        assert!(log.integrity.is_clean());
        assert!(
            log.times.windows(2).all(|w| w[1] > w[0]),
            "the decoded time axis must never fold back"
        );
        // Every sample is placed within half a nominal interval of its true time.
        for (i, time) in log.times.iter().enumerate() {
            assert!(
                (time - i as Real * 0.01).abs() <= 0.0055,
                "sample {} landed at {}",
                i,
                time
            );
        }
    }

    #[test]
    fn ordinary_jitter_does_not_fragment_a_record() {
        // The split only happens for a real discontinuity, so a few microseconds
        // of host clock jitter does not turn one record into two hundred.
        let mut writer = BinaryWriter::new(LogHeader::new(0.01), descriptors(1));
        for i in 0..200 {
            let jitter = if i % 2 == 0 { 2e-5 } else { -2e-5 };
            writer
                .push_sample(i as Real * 0.01 + jitter, &[i as Real])
                .unwrap();
        }
        let bytes = writer.finish().unwrap();
        let log = BinaryReader::read_all(&bytes).unwrap();
        assert_eq!(log.integrity.records_read, 1);
        assert_eq!(log.sample_count(), 200);
    }

    #[test]
    fn a_log_without_a_nominal_interval_never_splits_on_time() {
        let mut writer = BinaryWriter::new(LogHeader::new(0.0), descriptors(1));
        for i in 0..10 {
            writer.push_sample(i as Real * 100.0, &[i as Real]).unwrap();
        }
        assert_eq!(writer.buffered_samples(), 10);
        assert_eq!(writer.gap_tolerance_seconds, 0.0);
    }

    #[test]
    fn writer_rejects_a_sample_with_the_wrong_width() {
        let mut writer = BinaryWriter::new(LogHeader::new(0.01), descriptors(3));
        let err = writer.push_sample(0.0, &[1.0, 2.0]).unwrap_err();
        assert!(matches!(
            err,
            BinaryDecodeError::PayloadLengthMismatch { .. }
        ));
    }

    #[test]
    fn writer_clamps_samples_per_record_to_the_payload_field() {
        // 300 channels cannot fit 256 samples in a 16-bit payload length.
        let mut writer = BinaryWriter::new(LogHeader::new(0.01), descriptors(300));
        let values = vec![0.0; 300];
        for i in 0..40 {
            writer.push_sample(i as Real, &values).unwrap();
        }
        let bytes = writer.finish().unwrap();
        let log = BinaryReader::read_all(&bytes).unwrap();
        assert_eq!(log.sample_count(), 40);
        assert!(log.integrity.is_clean());
        assert!(log.integrity.records_read > 1);
    }

    #[test]
    fn stream_reader_delivers_records_one_at_a_time() {
        let bytes = write_log(6, 2, 2);
        let mut reader = LogStreamReader::new(&bytes[..]);
        reader.prepare().unwrap();
        assert_eq!(reader.channels().len(), 2);
        assert_eq!(reader.header().unwrap().device_id, "TESTDEV");

        let mut seen = 0usize;
        while let Some(record) = reader.next_record().unwrap() {
            assert_eq!(record.sample_count, 2);
            assert_eq!(record.values.len(), 4);
            assert_eq!(record.sequence, seen as u32);
            seen += 1;
        }
        assert_eq!(seen, 3);
        assert!(reader.integrity().is_clean());
    }

    #[test]
    fn empty_payload_log_reads_with_no_samples() {
        let writer = BinaryWriter::new(LogHeader::new(0.01), descriptors(2));
        let bytes = writer.finish().unwrap();
        assert_eq!(bytes.len(), HEADER_LEN + 2 * DESCRIPTOR_LEN);
        let log = BinaryReader::read_all(&bytes).unwrap();
        assert_eq!(log.sample_count(), 0);
        assert_eq!(log.integrity.records_read, 0);
    }

    #[test]
    fn descriptors_round_trip_names_roles_and_scales() {
        let mut channel = Channel::new("accel_x", ChannelRole::AccelX, vec![1.0]);
        channel.scale = hex_core::UnitScale::new(9.80665, 0.0);
        channel.sign = -1.0;
        let descriptor = ChannelDescriptor::from_channel(&channel);
        assert_eq!(descriptor.name_string(), "accel_x");
        assert_eq!(descriptor.role(), ChannelRole::AccelX);
        assert_eq!(descriptor.unit_code, unit_code_for("m/s^2"));
        assert!((descriptor.apply(1.0) + 9.80665).abs() < 1e-12);

        let bytes = BinaryWriter::new(LogHeader::new(0.01), vec![descriptor])
            .finish()
            .unwrap();
        let log = BinaryReader::read_all(&bytes).unwrap();
        assert_eq!(log.channels[0].name, "accel_x");
        assert_eq!(log.channels[0].role, ChannelRole::AccelX);
        assert_eq!(log.channels[0].unit, "m/s^2");
        assert_eq!(
            unit_for_code(log.channels[0].unit_code_round_trip()),
            "m/s^2"
        );
    }

    #[test]
    fn unit_codes_cover_the_common_units() {
        for (unit, code) in [
            ("s", 1u8),
            ("m/s^2", 2),
            ("rad/s", 3),
            ("T", 4),
            ("Pa", 5),
            ("K", 6),
            ("deg", 7),
            ("m", 8),
            ("m/s", 9),
            ("V", 10),
            ("A", 11),
            ("1", 12),
        ] {
            assert_eq!(unit_code_for(unit), code, "{unit}");
            assert_eq!(unit_for_code(code), unit);
        }
        assert_eq!(unit_code_for("furlongs"), UNIT_CODE_UNKNOWN);
        assert_eq!(unit_for_code(UNIT_CODE_UNKNOWN), "");
    }

    #[test]
    fn fixed_width_strings_truncate_on_a_character_boundary() {
        let encoded = encode_fixed_array("naive-cafe-\u{00e9}\u{00e9}\u{00e9}\u{00e9}", 32);
        assert_eq!(encoded.len(), 32);
        let round = decode_fixed_string(&encoded);
        assert!(round.starts_with("naive-cafe-"));

        let long = "x".repeat(100);
        let encoded = encode_fixed_array(&long, 32);
        assert_eq!(encoded.len(), 32);
        assert_eq!(decode_fixed_string(&encoded).len(), 32);
    }

    #[test]
    fn integrity_summary_mentions_the_counters() {
        let bytes = write_log(4, 2, 2);
        let log = BinaryReader::read_all(&bytes).unwrap();
        let summary = log.integrity.summary();
        assert!(summary.contains("2 record(s)"));
        assert!(summary.contains("4 sample(s)"));
    }

    #[test]
    fn channel_index_lookup_is_case_insensitive() {
        let bytes = write_log(2, 3, 256);
        let log = BinaryReader::read_all(&bytes).unwrap();
        assert_eq!(log.channel_index("CH1"), Some(1));
        assert_eq!(log.channel_index("nope"), None);
    }

    #[test]
    fn the_default_record_cap_is_256_samples() {
        let writer = BinaryWriter::new(LogHeader::new(0.01), descriptors(2));
        assert_eq!(
            writer.max_samples_per_record,
            DEFAULT_MAX_SAMPLES_PER_RECORD
        );
        assert_eq!(writer.buffered_samples(), 0);
        assert_eq!(writer.channels().len(), 2);
        assert_eq!(writer.header().channel_count, 2);
        assert_eq!(writer.header().magic, MAGIC);
        assert_eq!(writer.header().format_version, CURRENT_FORMAT_VERSION);
    }

    #[test]
    fn write_to_file_round_trips_through_disk() {
        let directory = std::env::temp_dir().join("hex-flight-data-tests");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("writer-round-trip.hlog");

        let mut writer = BinaryWriter::new(LogHeader::new(0.02), descriptors(2));
        for i in 0..40 {
            writer
                .push_sample(i as Real * 0.02, &[i as Real, -(i as Real)])
                .unwrap();
        }
        let written = writer.write_to_file(&path).unwrap();
        assert_eq!(written as u64, std::fs::metadata(&path).unwrap().len());

        let log = BinaryReader::read_file(&path).unwrap();
        assert_eq!(log.sample_count(), 40);
        assert_eq!(log.columns[1][7], -7.0);
        assert!(log.integrity.is_clean());
        let _ = std::fs::remove_file(&path);
    }
}
