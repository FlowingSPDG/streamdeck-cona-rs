//! TCP connection management

use crate::error::{Error, Result};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tokio::time::timeout;

/// Fixed packet size for TCP protocol
const PACKET_SIZE: usize = 1024;

/// Timeout for keep-alive responses (5 seconds as per protocol)
const KEEP_ALIVE_TIMEOUT: Duration = Duration::from_secs(5);

/// TCP connection wrapper for Stream Deck Studio
pub struct TcpConnection {
    stream: Arc<RwLock<TcpStream>>,
    last_data_time: Arc<RwLock<Instant>>,
    serial_number: Arc<RwLock<Option<String>>>,
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
        };

        // Request serial number to establish connection
        conn.request_serial().await?;

        // Start keep-alive monitoring task
        conn.start_keep_alive_monitor();

        Ok(conn)
    }

    /// Request serial number from the device
    async fn request_serial(&self) -> Result<()> {
        // Send serial number request: 0x03 0x84
        let mut packet = [0u8; PACKET_SIZE];
        packet[0] = 0x03;
        packet[1] = 0x84;

        self.send_packet(&packet).await?;

        // Read response until we get the serial number
        loop {
            let response = self.read_packet().await?;

            // Check if this is a serial number response (0x03 0x84)
            if response[0] == 0x03 && response[1] == 0x84 {
                // Length is in bytes 2-3 (Big Endian, 16-bit)
                let length = u16::from_be_bytes([response[2], response[3]]) as usize;

                if length > 0 && response.len() >= 4 + length {
                    let serial = String::from_utf8_lossy(&response[4..4 + length]).to_string();
                    *self.serial_number.write().await = Some(serial.clone());
                    log::debug!("Serial number: {}", serial);
                    return Ok(());
                }
            }
        }
    }

    /// Get the serial number (if available)
    pub async fn serial_number(&self) -> Option<String> {
        self.serial_number.read().await.clone()
    }

    /// Send a 1024-byte packet
    pub async fn send_packet(&self, packet: &[u8; PACKET_SIZE]) -> Result<()> {
        let mut stream = self.stream.write().await;
        stream
            .write_all(packet)
            .await
            .map_err(|e| Error::Io(e))?;
        stream
            .flush()
            .await
            .map_err(|e| Error::Io(e))?;
        Ok(())
    }

    /// Read a 1024-byte packet
    /// 
    /// This method reads up to 1024 bytes. If fewer bytes are received,
    /// the remaining bytes are zero-padded (matching Go implementation behavior).
    /// Go uses conn.Read() which can read partial data, so we do the same.
    pub async fn read_packet(&self) -> Result<[u8; PACKET_SIZE]> {
        let mut stream = self.stream.write().await;
        let mut packet = [0u8; PACKET_SIZE];

        // Read as much as we can (up to 1024 bytes)
        // Go implementation uses conn.Read() which can read partial data
        let n = stream.read(&mut packet).await.map_err(|e| {
            log::error!("Read error: {} (kind: {:?})", e, e.kind());
            Error::Io(e)
        })?;

        // If we got 0 bytes, it's an EOF
        if n == 0 {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "Connection closed by peer (0 bytes received)"
            )));
        }

        // Log if we received partial data (for debugging 0x43 0x93 issue)
        if n > 0 && n < PACKET_SIZE {
            log::debug!(
                "Received partial packet: {} bytes (expected 1024). First bytes: {:02x} {:02x}",
                n,
                packet[0],
                if n > 1 { packet[1] } else { 0 }
            );
            if n >= 2 {
                log::debug!("First 2 bytes of partial packet: 0x{:02x} 0x{:02x}", packet[0], packet[1]);
            }
        }

        // Update last data time
        *self.last_data_time.write().await = Instant::now();

        // Note: packet buffer is already zero-initialized, so partial reads are padded with zeros
        Ok(packet)
    }

    /// Read a packet with timeout
    pub async fn read_packet_timeout(
        &self,
        duration: Duration,
    ) -> Result<[u8; PACKET_SIZE]> {
        timeout(duration, self.read_packet())
            .await
            .map_err(|_| Error::Timeout(format!("No data received within {:?}", duration)))?
    }

    /// Handle keep-alive packet from device
    ///
    /// When device sends keep-alive (0x01 0x0a), client must respond with (0x03 0x1a)
    pub async fn handle_keep_alive(&self) -> Result<()> {
        let mut response = [0u8; PACKET_SIZE];
        response[0] = 0x03;
        response[1] = 0x1a;
        self.send_packet(&response).await?;
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
