//! Serial port enumeration and connection settings.
//!
//! Port listing and opening are kept behind a small surface so the rest of the
//! telemetry pipeline can be tested without hardware. Nothing here writes to a
//! device: the MVP is read-only by design, and a future command-capable mode
//! would need to be a separate, explicitly confirmed path.

use hex_core::Real;
use serde::{Deserialize, Serialize};

/// The baud rates the UI offers. Devices may use others; the settings type
/// carries a plain number so an unusual rate is still expressible.
pub const COMMON_BAUD_RATES: [u32; 10] = [
    9600, 19200, 38400, 57600, 115200, 230400, 250_000, 460_800, 500_000, 921_600,
];

/// Serial line parameters for one device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SerialSettings {
    pub baud_rate: u32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub parity: Parity,
    pub flow_control: FlowControl,
    /// Read timeout in milliseconds. Zero means block until data arrives.
    pub read_timeout_ms: u64,
}

impl Default for SerialSettings {
    fn default() -> Self {
        Self {
            baud_rate: 115_200,
            data_bits: 8,
            stop_bits: 1,
            parity: Parity::None,
            flow_control: FlowControl::None,
            read_timeout_ms: 50,
        }
    }
}

impl SerialSettings {
    /// Settings for a baud rate with the usual 8N1 framing.
    pub fn at_baud(baud_rate: u32) -> Self {
        Self {
            baud_rate,
            ..Self::default()
        }
    }

    /// Approximate maximum bytes per second for this line, used to size buffers
    /// and to warn when a configured packet rate cannot physically fit.
    pub fn maximum_bytes_per_second(&self) -> Real {
        // One start bit, `data_bits` data bits, optional parity, `stop_bits`.
        let parity_bits = match self.parity {
            Parity::None => 0,
            _ => 1,
        };
        let bits_per_byte = 1 + self.data_bits as u32 + parity_bits + self.stop_bits as u32;
        if bits_per_byte == 0 {
            return 0.0;
        }
        self.baud_rate as Real / bits_per_byte as Real
    }

    pub fn label(&self) -> String {
        let parity = match self.parity {
            Parity::None => 'N',
            Parity::Odd => 'O',
            Parity::Even => 'E',
        };
        format!(
            "{} baud, {}{}{}{}",
            self.baud_rate,
            self.data_bits,
            parity,
            self.stop_bits,
            match self.flow_control {
                FlowControl::None => "",
                FlowControl::Software => " XON/XOFF",
                FlowControl::Hardware => " RTS/CTS",
            }
        )
    }
}

/// Serial parity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Parity {
    None,
    Odd,
    Even,
}

/// Serial flow control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowControl {
    None,
    Software,
    Hardware,
}

/// A serial port as reported by the operating system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortInfo {
    /// System name, for example `COM4`.
    pub port_name: String,
    /// Human-readable description when the driver provides one.
    pub description: Option<String>,
    /// Manufacturer string when available.
    pub manufacturer: Option<String>,
    /// USB product name when available.
    pub product: Option<String>,
    /// USB serial number when available.
    pub serial_number: Option<String>,
    /// USB vendor and product identifiers when available.
    pub usb_ids: Option<(u16, u16)>,
    /// Whether the port is a USB device rather than a legacy UART.
    pub is_usb: bool,
}

impl PortInfo {
    pub fn new(port_name: impl Into<String>) -> Self {
        Self {
            port_name: port_name.into(),
            description: None,
            manufacturer: None,
            product: None,
            serial_number: None,
            usb_ids: None,
            is_usb: false,
        }
    }

    /// A label for the port picker: the name plus whatever detail is available.
    pub fn label(&self) -> String {
        match (&self.description, &self.product) {
            (Some(d), _) => format!("{} - {}", self.port_name, d),
            (None, Some(p)) => format!("{} - {}", self.port_name, p),
            _ => self.port_name.clone(),
        }
    }

    /// A stable identity for a device profile.
    ///
    /// A USB serial number survives being plugged into a different port, which is
    /// what a device profile needs to keep matching its hardware. Falling back to
    /// the port name is unavoidable for a device without one.
    pub fn identity(&self) -> String {
        match (&self.serial_number, &self.usb_ids) {
            (Some(sn), Some((vid, pid))) => format!("usb:{:04x}:{:04x}:{}", vid, pid, sn),
            (Some(sn), None) => format!("usb:{}", sn),
            _ => format!("port:{}", self.port_name),
        }
    }
}

/// The connection state machine the UI displays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    /// No port is open.
    Disconnected,
    /// A port open is in progress.
    Connecting,
    /// The port is open but no valid packet has arrived yet.
    Connected,
    /// Valid packets are arriving.
    Receiving,
    /// The port is open but no packet has arrived within the staleness window.
    Stalled,
    /// The last operation failed. The detail is in the health report.
    Error,
}

impl ConnectionState {
    pub fn label(self) -> &'static str {
        match self {
            ConnectionState::Disconnected => "Disconnected",
            ConnectionState::Connecting => "Connecting",
            ConnectionState::Connected => "Connected",
            ConnectionState::Receiving => "Receiving",
            ConnectionState::Stalled => "Stalled",
            ConnectionState::Error => "Error",
        }
    }

    /// Whether the transport is open in this state.
    pub fn is_open(self) -> bool {
        matches!(
            self,
            ConnectionState::Connected | ConnectionState::Receiving | ConnectionState::Stalled
        )
    }
}

/// How the connection is doing, recomputed from observed traffic.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ConnectionHealth {
    /// Bytes received since the connection opened.
    pub bytes_received: u64,
    /// Valid packets decoded.
    pub packets_received: u64,
    /// Packets rejected by integrity or format checks.
    pub packets_rejected: u64,
    /// Frames lost, inferred from sequence numbers.
    pub packets_dropped: u64,
    /// CRC or checksum failures.
    pub crc_failures: u64,
    /// Bytes discarded while resynchronising.
    pub discarded_bytes: u64,
    /// Time of the most recent valid packet, seconds since the session start.
    pub last_packet_time: Option<Real>,
}

impl ConnectionHealth {
    /// Packets per second over a window.
    pub fn packet_rate(&self, window_seconds: Real) -> Real {
        if window_seconds <= 0.0 {
            0.0
        } else {
            self.packets_received as Real / window_seconds
        }
    }

    /// Bytes per second over a window.
    pub fn byte_rate(&self, window_seconds: Real) -> Real {
        if window_seconds <= 0.0 {
            0.0
        } else {
            self.bytes_received as Real / window_seconds
        }
    }

    /// Fraction of packets that failed a check.
    pub fn rejection_ratio(&self) -> Real {
        let total = self.packets_received + self.packets_rejected;
        if total == 0 {
            0.0
        } else {
            self.packets_rejected as Real / total as Real
        }
    }

    /// Whether the stream looks stalled, given the current time and a window.
    pub fn is_stalled(&self, now: Real, stall_seconds: Real) -> bool {
        match self.last_packet_time {
            Some(t) => now - t > stall_seconds,
            None => true,
        }
    }

    /// Decide the state from the health and the current time.
    pub fn state(&self, now: Real, stall_seconds: Real) -> ConnectionState {
        if self.packets_received == 0 {
            return ConnectionState::Connected;
        }
        if self.is_stalled(now, stall_seconds) {
            ConnectionState::Stalled
        } else {
            ConnectionState::Receiving
        }
    }
}

/// A byte source the pipeline can read from.
///
/// Implemented by the real serial transport and by a scripted transport used in
/// tests, so every stage above the bytes can be exercised without hardware.
pub trait ByteSource: Send {
    /// Read whatever bytes are available into `buffer`, returning how many.
    ///
    /// Returning `Ok(0)` means nothing was available this call, not end of
    /// stream.
    fn read_available(&mut self, buffer: &mut [u8]) -> std::io::Result<usize>;

    /// Whether the source has no more data and never will.
    ///
    /// A serial port never reports exhaustion, so the default is `false`. A file
    /// or a scripted stream does, which is what lets a reader loop terminate
    /// instead of waiting for data that cannot arrive.
    fn is_exhausted(&self) -> bool {
        false
    }

    /// A description for the health panel, for example `COM4 at 115200 baud`.
    fn describe(&self) -> String;

    /// Close the source. Must be idempotent.
    fn close(&mut self);
}

/// A byte source backed by a `Vec<u8>` script, for tests and replay.
#[derive(Debug, Clone, Default)]
pub struct ScriptedSource {
    data: Vec<u8>,
    position: usize,
    /// Maximum bytes returned by one read, so a test can exercise partial reads.
    pub chunk: usize,
    description: String,
}

impl ScriptedSource {
    pub fn new(data: impl Into<Vec<u8>>) -> Self {
        Self {
            data: data.into(),
            position: 0,
            chunk: 64,
            description: "scripted source".to_string(),
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn with_chunk(mut self, chunk: usize) -> Self {
        self.chunk = chunk.max(1);
        self
    }

    /// Bytes not yet handed out.
    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.position)
    }
}

impl ByteSource for ScriptedSource {
    fn read_available(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.position >= self.data.len() {
            return Ok(0);
        }
        let take = buffer
            .len()
            .min(self.chunk)
            .min(self.data.len() - self.position);
        buffer[..take].copy_from_slice(&self.data[self.position..self.position + take]);
        self.position += take;
        Ok(take)
    }

    fn is_exhausted(&self) -> bool {
        self.position >= self.data.len()
    }

    fn describe(&self) -> String {
        self.description.clone()
    }

    fn close(&mut self) {
        self.position = self.data.len();
    }
}

/// The real serial transport.
#[cfg(not(target_arch = "wasm32"))]
pub struct SerialByteSource {
    port: Box<dyn serialport::SerialPort>,
    description: String,
    closed: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl SerialByteSource {
    /// Open a port using the supplied settings.
    ///
    /// The port is opened read-only in effect: nothing in this crate writes to
    /// it, which is the safety default for the MVP.
    pub fn open(port_name: &str, settings: &SerialSettings) -> Result<Self, TransportError> {
        use serialport::{DataBits, FlowControl as SpFlow, Parity as SpParity, StopBits};

        let data_bits = match settings.data_bits {
            5 => DataBits::Five,
            6 => DataBits::Six,
            7 => DataBits::Seven,
            _ => DataBits::Eight,
        };
        let parity = match settings.parity {
            Parity::None => SpParity::None,
            Parity::Odd => SpParity::Odd,
            Parity::Even => SpParity::Even,
        };
        let stop_bits = if settings.stop_bits == 2 {
            StopBits::Two
        } else {
            StopBits::One
        };
        let flow_control = match settings.flow_control {
            FlowControl::None => SpFlow::None,
            FlowControl::Software => SpFlow::Software,
            FlowControl::Hardware => SpFlow::Hardware,
        };

        let port = serialport::new(port_name, settings.baud_rate)
            .data_bits(data_bits)
            .parity(parity)
            .stop_bits(stop_bits)
            .flow_control(flow_control)
            .timeout(std::time::Duration::from_millis(settings.read_timeout_ms))
            .open()
            .map_err(|e| TransportError::OpenFailed {
                port: port_name.to_string(),
                detail: e.to_string(),
            })?;

        Ok(Self {
            port,
            description: format!("{} at {}", port_name, settings.label()),
            closed: false,
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl ByteSource for SerialByteSource {
    fn read_available(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self.port.read(buffer) {
            Ok(n) => Ok(n),
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Ok(0),
            Err(e) => Err(e),
        }
    }

    fn describe(&self) -> String {
        self.description.clone()
    }

    fn close(&mut self) {
        self.closed = true;
    }
}

/// A failure while opening or enumerating ports.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    #[error("Unable to enumerate serial ports: {detail}")]
    EnumerationFailed { detail: String },
    #[error("Unable to open {port}: {detail}")]
    OpenFailed { port: String, detail: String },
    #[error("The port {port} was not found. It may have been unplugged.")]
    PortNotFound { port: String },
    #[error("Serial support is unavailable on this platform.")]
    Unsupported,
}

impl TransportError {
    /// What the user should try next.
    pub fn suggested_action(&self) -> &'static str {
        match self {
            TransportError::EnumerationFailed { .. } => {
                "Reconnect the device and refresh the port list."
            }
            TransportError::OpenFailed { .. } => {
                "Check that no other program has the port open, and that the baud rate matches the device."
            }
            TransportError::PortNotFound { .. } => "Refresh the port list and select the device again.",
            TransportError::Unsupported => "Install the platform serial driver and try again.",
        }
    }
}

/// List the serial ports the operating system reports.
#[cfg(not(target_arch = "wasm32"))]
pub fn list_ports() -> Result<Vec<PortInfo>, TransportError> {
    let ports = serialport::available_ports().map_err(|e| TransportError::EnumerationFailed {
        detail: e.to_string(),
    })?;

    Ok(ports
        .into_iter()
        .map(|p| {
            let mut info = PortInfo::new(p.port_name.clone());
            match p.port_type {
                serialport::SerialPortType::UsbPort(usb) => {
                    info.is_usb = true;
                    info.manufacturer = usb.manufacturer.clone();
                    info.product = usb.product.clone();
                    info.serial_number = usb.serial_number.clone();
                    info.usb_ids = Some((usb.vid, usb.pid));
                    if info.description.is_none() {
                        info.description = usb.product.clone();
                    }
                }
                serialport::SerialPortType::PciPort => {
                    info.description = Some("PCI serial port".to_string());
                }
                serialport::SerialPortType::BluetoothPort => {
                    info.description = Some("Bluetooth serial port".to_string());
                }
                serialport::SerialPortType::Unknown => {
                    info.description = Some("Serial port".to_string());
                }
            }
            info
        })
        .collect())
}

/// Stub port list on platforms without serial support.
#[cfg(target_arch = "wasm32")]
pub fn list_ports() -> Result<Vec<PortInfo>, TransportError> {
    Err(TransportError::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_are_8n1_at_115200() {
        let s = SerialSettings::default();
        assert_eq!(s.baud_rate, 115_200);
        assert_eq!(s.data_bits, 8);
        assert_eq!(s.stop_bits, 1);
        assert_eq!(s.parity, Parity::None);
        assert!(s.label().contains("115200"));
        assert!(s.label().contains("8N1"));
    }

    #[test]
    fn byte_rate_accounts_for_framing_bits() {
        let s = SerialSettings::at_baud(115_200);
        // 10 bits per byte at 8N1.
        assert!((s.maximum_bytes_per_second() - 11_520.0).abs() < 1e-9);
        let two_stop = SerialSettings { stop_bits: 2, ..s };
        assert!(two_stop.maximum_bytes_per_second() < s.maximum_bytes_per_second());
    }

    #[test]
    fn port_identity_prefers_the_usb_serial_number() {
        let mut p = PortInfo::new("COM4");
        assert_eq!(p.identity(), "port:COM4");
        p.serial_number = Some("ABC123".to_string());
        assert_eq!(p.identity(), "usb:ABC123");
        p.usb_ids = Some((0x2341, 0x0043));
        assert_eq!(p.identity(), "usb:2341:0043:ABC123");
    }

    #[test]
    fn port_label_includes_available_detail() {
        let mut p = PortInfo::new("COM4");
        assert_eq!(p.label(), "COM4");
        p.product = Some("Arduino Uno".to_string());
        assert_eq!(p.label(), "COM4 - Arduino Uno");
        p.description = Some("USB Serial Device".to_string());
        assert_eq!(p.label(), "COM4 - USB Serial Device");
    }

    #[test]
    fn connection_state_open_predicate() {
        assert!(!ConnectionState::Disconnected.is_open());
        assert!(!ConnectionState::Error.is_open());
        assert!(ConnectionState::Connected.is_open());
        assert!(ConnectionState::Receiving.is_open());
        assert!(ConnectionState::Stalled.is_open());
        assert_eq!(ConnectionState::Receiving.label(), "Receiving");
    }

    #[test]
    fn health_reports_rates_and_ratios() {
        let h = ConnectionHealth {
            bytes_received: 20_000,
            packets_received: 1_000,
            packets_rejected: 10,
            ..Default::default()
        };
        assert!((h.packet_rate(10.0) - 100.0).abs() < 1e-9);
        assert!((h.byte_rate(10.0) - 2_000.0).abs() < 1e-9);
        assert!((h.rejection_ratio() - 10.0 / 1010.0).abs() < 1e-12);
    }

    #[test]
    fn health_stall_detection() {
        let h = ConnectionHealth {
            packets_received: 5,
            last_packet_time: Some(10.0),
            ..Default::default()
        };
        assert!(!h.is_stalled(10.5, 2.0));
        assert!(h.is_stalled(20.0, 2.0));
        assert_eq!(h.state(20.0, 2.0), ConnectionState::Stalled);
        assert_eq!(h.state(10.1, 2.0), ConnectionState::Receiving);
    }

    #[test]
    fn health_with_no_packets_is_merely_connected() {
        let h = ConnectionHealth::default();
        assert!(h.is_stalled(0.0, 1.0));
        assert_eq!(h.state(0.0, 1.0), ConnectionState::Connected);
        assert_eq!(h.packet_rate(0.0), 0.0);
        assert_eq!(h.rejection_ratio(), 0.0);
    }

    #[test]
    fn scripted_source_hands_out_data_in_chunks() {
        let mut src = ScriptedSource::new(vec![1u8, 2, 3, 4, 5]).with_chunk(2);
        let mut buf = [0u8; 8];
        assert_eq!(src.read_available(&mut buf).unwrap(), 2);
        assert_eq!(&buf[..2], &[1, 2]);
        assert_eq!(src.read_available(&mut buf).unwrap(), 2);
        assert_eq!(src.read_available(&mut buf).unwrap(), 1);
        assert_eq!(src.read_available(&mut buf).unwrap(), 0);
        assert_eq!(src.remaining(), 0);
        assert_eq!(src.describe(), "scripted source");
        src.close();
        assert_eq!(src.read_available(&mut buf).unwrap(), 0);
    }

    #[test]
    fn transport_error_actions_are_specific() {
        let e = TransportError::OpenFailed {
            port: "COM4".to_string(),
            detail: "Access is denied".to_string(),
        };
        assert!(e.to_string().contains("COM4"));
        assert!(!e.suggested_action().is_empty());
        let e = TransportError::PortNotFound {
            port: "COM9".to_string(),
        };
        assert!(e.to_string().contains("unplugged"));
    }

    #[test]
    fn common_baud_rates_are_ascending() {
        for w in COMMON_BAUD_RATES.windows(2) {
            assert!(w[1] > w[0]);
        }
    }
}
