//! TCP connection management with Cora protocol support

use crate::error::{Error, Result};
use streamdeck_cora_rs_core::{CoraMessage, CoraMessageFlags, CoraHidOp, CORA_MAGIC};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::RwLock;

/// Timeout for keep-alive responses (5 seconds as per protocol)
const KEEP_ALIVE_TIMEOUT: Duration = Duration::from_secs(5);

/// TCP connection wrapper for Stream Deck Studio (Cora protocol only)
pub struct TcpConnection {
    stream: Arc<RwLock<TcpStream>>,
    last_data_time: Arc<RwLock<Instant>>,
    serial_number: Arc<RwLock<Option<String>>>,
    receive_buffer: Arc<RwLock<Vec<u8>>>,
    next_message_id: Arc<RwLock<u32>>,
}

impl TcpConnection {
    /// Create a new TCP connection to a Stream Deck Studio
    ///
    /// # Arguments
    /// * `addr` - Address in format "IP:port" (e.g., "192.168.1.100:5343")
    ///
    /// # Returns
    /// Returns a `TcpConnection` instance after successfully connecting
    pub async fn connect(addr: impl AsRef<str>) -> Result<Self> {
        let addr = addr.as_ref();
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|e| Error::Connection(format!("Failed to connect to {}: {}", addr, e)))?;

        let conn = Self {
            stream: Arc::new(RwLock::new(stream)),
            last_data_time: Arc::new(RwLock::new(Instant::now())),
            serial_number: Arc::new(RwLock::new(None)),
            receive_buffer: Arc::new(RwLock::new(Vec::new())),
            next_message_id: Arc::new(RwLock::new(0)),
        };

        // Wait for keep-alive packet from device to confirm connection
        conn.handle_initial_keep_alive().await?;

        // Request serial number to establish connection
        conn.request_serial().await?;

        // Start keep-alive monitoring task
        conn.start_keep_alive_monitor();

        Ok(conn)
    }

    /// Handle initial keep-alive packet from device
    async fn handle_initial_keep_alive(&self) -> Result<()> {
        let message = self.read_cora_message().await?;

        if message.is_keep_alive() {
            // Send ACK response
            let connection_no = message.keep_alive_connection_no().unwrap_or(0);
            let ack_payload = {
                let mut payload = vec![0u8; 32];
                payload[0] = 0x03;
                payload[1] = 0x1a;
                payload[2] = connection_no;
                payload
            };

            let ack_message = CoraMessage::new(
                CoraMessageFlags::AckNak,
                message.hid_op,
                message.message_id,
                ack_payload,
            );

            self.send_cora_message(&ack_message).await?;
            log::debug!("Handled initial keep-alive, connection established");
            Ok(())
        } else {
            Err(Error::Protocol("Expected keep-alive packet as first message".to_string()))
        }
    }

    /// Request serial number from the device
    async fn request_serial(&self) -> Result<()> {
        // Send GET_REPORT request for serial number (0x84)
        let payload = vec![0x03, 0x84];
        let message_id = {
            let mut id = self.next_message_id.write().await;
            *id += 1;
            *id
        };

        let message = CoraMessage::new(
            CoraMessageFlags::None,
            CoraHidOp::GetReport,
            message_id,
            payload,
        );

        self.send_cora_message(&message).await?;

        // Read response
        loop {
            let response = self.read_cora_message().await?;

            if response.hid_op == CoraHidOp::GetReport
                && (response.flags == CoraMessageFlags::Result || response.flags == CoraMessageFlags::None)
                && response.payload.len() >= 2
                && response.payload[0] == 0x03
                && response.payload[1] == 0x84
            {
                // Length is in bytes 2-3 (Big Endian, 16-bit)
                if response.payload.len() >= 4 {
                    let length = u16::from_be_bytes([response.payload[2], response.payload[3]]) as usize;

                    if response.payload.len() >= 4 + length && length > 0 {
                        let serial = String::from_utf8_lossy(&response.payload[4..4 + length]).to_string();
                        *self.serial_number.write().await = Some(serial.clone());
                        log::debug!("Serial number: {}", serial);
                        return Ok(());
                    }
                }
            }
        }
    }

    /// Get the serial number (if available)
    pub async fn serial_number(&self) -> Option<String> {
        self.serial_number.read().await.clone()
    }

    /// Send a Cora protocol message
    pub async fn send_cora_message(&self, message: &CoraMessage) -> Result<()> {
        let data = message.encode();
        let mut stream = self.stream.write().await;
        stream
            .write_all(&data)
            .await
            .map_err(|e| Error::Io(e))?;
        stream
            .flush()
            .await
            .map_err(|e| Error::Io(e))?;
        Ok(())
    }

    /// Send command payload as Cora message (automatically determines HID operation and generates message ID)
    pub async fn send_command_payload(&self, payload: Vec<u8>) -> Result<()> {
        let message_id = {
            let mut id = self.next_message_id.write().await;
            *id += 1;
            *id
        };

        // Determine HID operation based on payload
        let hid_op = if payload.len() >= 2 && payload[0] == 0x03 {
            // Feature report (0x03 prefix)
            CoraHidOp::SendReport
        } else {
            // HID write for images and other data
            CoraHidOp::Write
        };

        let message = CoraMessage::new(
            CoraMessageFlags::None,
            hid_op,
            message_id,
            payload,
        );

        self.send_cora_message(&message).await
    }

    /// Read a Cora protocol message
    pub async fn read_cora_message(&self) -> Result<CoraMessage> {
        let mut stream = self.stream.write().await;
        let mut buffer = self.receive_buffer.write().await;

        // If buffer doesn't start with magic, search for it
        if buffer.len() >= 4 {
            if let Some(pos) = buffer.windows(4).position(|w| w == &CORA_MAGIC) {
                if pos > 0 {
                    buffer.drain(..pos);
                }
            } else {
                // No magic found, keep last 3 bytes and read more
                if buffer.len() > 3 {
                    let keep = buffer[buffer.len() - 3..].to_vec();
                    buffer.clear();
                    buffer.extend_from_slice(&keep);
                }
            }
        }

        // Read until we have at least a header (16 bytes)
        while buffer.len() < 16 {
            let mut chunk = vec![0u8; 4096];
            let n = stream.read(&mut chunk).await
                .map_err(|e| Error::Io(e))?;

            if n == 0 {
                return Err(Error::Connection("Connection closed".to_string()));
            }

            buffer.extend_from_slice(&chunk[..n]);
            *self.last_data_time.write().await = Instant::now();
        }

        // Read payload length from header (bytes 12-15, Little Endian)
        let payload_length = u32::from_le_bytes([
            buffer[12], buffer[13], buffer[14], buffer[15]
        ]) as usize;

        // Read until we have the full message
        while buffer.len() < 16 + payload_length {
            let mut chunk = vec![0u8; 4096];
            let n = stream.read(&mut chunk).await
                .map_err(|e| Error::Io(e))?;

            if n == 0 {
                return Err(Error::Connection("Connection closed".to_string()));
            }

            buffer.extend_from_slice(&chunk[..n]);
            *self.last_data_time.write().await = Instant::now();
        }

        // Decode message
        let message = CoraMessage::decode(&buffer[..16 + payload_length])?;

        // Remove processed data from buffer
        buffer.drain(..16 + payload_length);

        Ok(message)
    }


    /// Handle keep-alive packet from device
    pub async fn handle_keep_alive(&self) -> Result<()> {
        let message = self.read_cora_message().await?;

        if message.is_keep_alive() {
            // Send ACK response
            let connection_no = message.keep_alive_connection_no().unwrap_or(0);
            let ack_payload = {
                let mut payload = vec![0u8; 32];
                payload[0] = 0x03;
                payload[1] = 0x1a;
                payload[2] = connection_no;
                payload
            };

            let ack_message = CoraMessage::new(
                CoraMessageFlags::AckNak,
                message.hid_op,
                message.message_id,
                ack_payload,
            );

            self.send_cora_message(&ack_message).await?;
        }

        Ok(())
    }

    /// Start background task to monitor keep-alive timeout
    fn start_keep_alive_monitor(&self) {
        let last_data_time = Arc::clone(&self.last_data_time);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));

            loop {
                interval.tick().await;

                let last_time = *last_data_time.read().await;
                if last_time.elapsed() > KEEP_ALIVE_TIMEOUT {
                    log::warn!(
                        "No data received within {} seconds, connection may be dead",
                        KEEP_ALIVE_TIMEOUT.as_secs()
                    );
                    // Note: We don't close the connection here as the user might want to handle it
                    // This is just a warning
                    break;
                }
            }
        });
    }

    /// Check if keep-alive timeout has been exceeded
    pub async fn is_keep_alive_timeout(&self) -> bool {
        self.last_data_time.read().await.elapsed() > KEEP_ALIVE_TIMEOUT
    }
}
