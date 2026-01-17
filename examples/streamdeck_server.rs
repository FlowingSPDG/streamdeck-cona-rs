//! Stream Deck Studio Server Emulator
//!
//! This example implements a TCP server that emulates a Stream Deck Studio device.
//! It can be used for testing clients or as a reference implementation.
//!
//! This emulator implements the Cora protocol (modern Stream Deck Studio/Plus).
//! Previously there was a Legacy protocol, but it is not supported here.

use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;

const DEFAULT_PORT: u16 = 5343;

// Cora protocol magic bytes
const CORA_MAGIC: [u8; 4] = [0x43, 0x93, 0x8a, 0x41];

#[derive(Debug, Clone)]
struct DeviceState {
    serial_number: String,
    brightness: u8,
    button_images: [Option<Vec<u8>>; 32],
    encoder_colors: [(u8, u8, u8); 2],
}

impl Default for DeviceState {
    fn default() -> Self {
        Self {
            serial_number: "EMULATOR123456".to_string(),
            brightness: 100,
            button_images: Default::default(),
            encoder_colors: [(0, 0, 0), (0, 0, 0)],
        }
    }
}

async fn handle_client(stream: TcpStream, state: Arc<RwLock<DeviceState>>) -> Result<(), Box<dyn std::error::Error>> {
    let (mut reader, mut writer) = stream.into_split();
    let mut buffer = vec![0u8; 4096]; // Buffer for reading data
    let mut receive_buffer = Vec::new();
    let mut protocol_detected = false;
    let mut _last_data_time = std::time::Instant::now();

    println!("[Server] Client connected");

    // Send initial keep-alive packet to establish Cora protocol
    // The client detects protocol mode by receiving data starting with CORA_MAGIC
    let mut init_keepalive_payload = vec![0u8; 32];
    init_keepalive_payload[0] = 0x01;
    init_keepalive_payload[1] = 0x0a;
    init_keepalive_payload[5] = 0x01; // connection no

    let mut init_keepalive_header = [0u8; 16];
    init_keepalive_header[..4].copy_from_slice(&CORA_MAGIC);
    init_keepalive_header[4..6].copy_from_slice(&0x0000u16.to_le_bytes()); // No flags
    init_keepalive_header[6] = 0x01; // SEND_REPORT
    init_keepalive_header[8..12].copy_from_slice(&1u32.to_le_bytes()); // message ID = 1
    init_keepalive_header[12..16].copy_from_slice(&(init_keepalive_payload.len() as u32).to_le_bytes());

    writer.write_all(&init_keepalive_header).await?;
    writer.write_all(&init_keepalive_payload).await?;
    println!("[Server] Sent initial keep-alive packet (Cora protocol)");

    loop {
        // Read data (can be partial)
        let n = match tokio::time::timeout(std::time::Duration::from_secs(10), reader.read(&mut buffer)).await {
            Ok(Ok(n)) => {
                if n == 0 {
                    eprintln!("[Server] Connection closed by client");
                    break;
                }
                _last_data_time = std::time::Instant::now();
                n
            }
            Ok(Err(e)) => {
                eprintln!("[Server] Read error: {} (kind: {:?})", e, e.kind());
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    eprintln!("[Server] Early EOF - received data: {:02x?}", &buffer[..std::cmp::min(buffer.len(), 64)]);
                }
                break;
            }
            Err(_) => {
                eprintln!("[Server] Read timeout");
                break;
            }
        };

        // Check if timeout exceeded
        if _last_data_time.elapsed() > std::time::Duration::from_secs(5) {
            eprintln!("[Server] Client timeout");
            break;
        }

        // Append received data to buffer
        receive_buffer.extend_from_slice(&buffer[..n]);

        // Debug: log first received bytes
        if receive_buffer.len() > 0 && !protocol_detected {
            println!("[Server] Received {} bytes, first bytes: {:02x?}", 
                receive_buffer.len(), &receive_buffer[..std::cmp::min(receive_buffer.len(), 16)]);
        }

        // Detect Cora protocol on first packet
        if !protocol_detected {
            // Check for Cora magic bytes
            if receive_buffer.len() >= 4 && receive_buffer.starts_with(&CORA_MAGIC) {
                protocol_detected = true;
                println!("[Server] Detected Cora protocol mode");
            } else {
                // Wait for more data to detect protocol
                if receive_buffer.len() < 4 {
                    println!("[Server] Waiting for more data to detect protocol (have {} bytes)", receive_buffer.len());
                    continue;
                }
                // Not Cora protocol - reject connection
                eprintln!("[Server] Non-Cora protocol detected - first bytes: {:02x?}", &receive_buffer[..std::cmp::min(receive_buffer.len(), 16)]);
                break;
            }
        }

        // Handle Cora protocol only
        if protocol_detected {
            // Cora message format: [16-byte header][variable payload]
            if receive_buffer.len() < 16 {
                continue; // Wait for full header
            }

            // Parse Cora header
            let payload_length = u32::from_le_bytes([
                receive_buffer[12],
                receive_buffer[13],
                receive_buffer[14],
                receive_buffer[15],
            ]) as usize;

            if receive_buffer.len() < 16 + payload_length {
                continue; // Wait for full message
            }

            let flags = u16::from_le_bytes([receive_buffer[4], receive_buffer[5]]);
            let hid_op = receive_buffer[6];
            let message_id = u32::from_le_bytes([
                receive_buffer[8],
                receive_buffer[9],
                receive_buffer[10],
                receive_buffer[11],
            ]);
            let payload = &receive_buffer[16..16 + payload_length];

            // Remove processed message from buffer
            receive_buffer.drain(..16 + payload_length);

            println!("[Server] Cora message: flags=0x{:04x}, hid_op={}, msg_id={}, payload_len={}", 
                flags, hid_op, message_id, payload_length);

            // Handle keep-alive in Cora protocol
            if payload.len() > 4 && payload[0] == 0x01 && payload[1] == 0x0a {
                // Keep-alive response
                let mut ack_payload = vec![0u8; 32];
                ack_payload[0] = 0x03;
                ack_payload[1] = 0x1a;
                if payload.len() > 5 {
                    ack_payload[2] = payload[5]; // connection no
                }

                // Send Cora ACK message
                let mut ack_header = [0u8; 16];
                ack_header[..4].copy_from_slice(&CORA_MAGIC);
                ack_header[4..6].copy_from_slice(&0x0200u16.to_le_bytes()); // ACK_NAK flag
                ack_header[6] = hid_op;
                ack_header[8..12].copy_from_slice(&message_id.to_le_bytes());
                ack_header[12..16].copy_from_slice(&(ack_payload.len() as u32).to_le_bytes());

                writer.write_all(&ack_header).await?;
                writer.write_all(&ack_payload).await?;
                println!("[Server] Sent Cora keep-alive ACK");
            } else if hid_op == 0x02 {
                // GET_REPORT request (0x02)
                // Determine if primary or secondary port based on VERBATIM flag
                let is_primary = (flags & 0x8000) == 0; // VERBATIM = 0x8000 means secondary
                let report_id = if is_primary && payload.len() > 1 && payload[0] == 0x03 {
                    payload[1]
                } else if payload.len() > 0 {
                    payload[0]
                } else {
                    0
                };

                let mut response_payload = Vec::new();

                match report_id {
                    0x80 | 0x83 => {
                        // Firmware version request
                        let version = "1.05.008";
                        response_payload = vec![0u8; 32];
                        if is_primary {
                            response_payload[0] = 0x03;
                            response_payload[1] = report_id;
                        } else {
                            response_payload[0] = report_id;
                        }
                        let version_bytes = version.as_bytes();
                        let offset = if is_primary { 8 } else { 5 };
                        if response_payload.len() > offset + version_bytes.len() {
                            response_payload[offset..offset + version_bytes.len()].copy_from_slice(version_bytes);
                        }
                        println!("[Server] Cora GET_REPORT: firmware version (0x{:02x}), primary={}", report_id, is_primary);
                    }
                    0x84 => {
                        // Serial number request
                        let serial = state.read().await.serial_number.clone();
                        response_payload = vec![0u8; 32];
                        if is_primary {
                            response_payload[0] = 0x03;
                            response_payload[1] = 0x84;
                        } else {
                            response_payload[0] = 0x84;
                        }
                        let len = serial.len().min(32 - 4) as u16;
                        response_payload[2] = ((len >> 8) & 0xff) as u8; // Big Endian
                        response_payload[3] = (len & 0xff) as u8;
                        let copy_len = len.min(32 - 4) as usize;
                        if response_payload.len() > 4 && serial.len() >= copy_len {
                            response_payload[4..4 + copy_len].copy_from_slice(&serial.as_bytes()[..copy_len]);
                        }
                        println!("[Server] Cora GET_REPORT: serial number (0x84), primary={}", is_primary);
                    }
                    _ => {
                        // Unknown report ID
                        response_payload = vec![0u8; 32];
                        if is_primary {
                            response_payload[0] = 0x03;
                            response_payload[1] = report_id;
                        } else {
                            response_payload[0] = report_id;
                        }
                        println!("[Server] Cora GET_REPORT: unknown report ID (0x{:02x}), primary={}", report_id, is_primary);
                    }
                }

                // Send Cora response with RESULT flag
                let mut response_header = [0u8; 16];
                response_header[..4].copy_from_slice(&CORA_MAGIC);
                response_header[4..6].copy_from_slice(&0x0100u16.to_le_bytes()); // RESULT flag
                response_header[6] = hid_op;
                response_header[8..12].copy_from_slice(&message_id.to_le_bytes());
                response_header[12..16].copy_from_slice(&(response_payload.len() as u32).to_le_bytes());

                writer.write_all(&response_header).await?;
                writer.write_all(&response_payload).await?;
            } else {
                // Handle other Cora messages (for now, just log)
                println!("[Server] Cora message: hid_op={}, payload={:02x?}", 
                    hid_op, &payload[..std::cmp::min(payload.len(), 16)]);
            }

            continue;
        }

        // Simulate button events occasionally (for testing)
        // In a real device, this would come from hardware
    }

    println!("[Server] Client disconnected");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let port = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(DEFAULT_PORT);

    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&addr).await?;

    println!("Stream Deck Studio Emulator");
    println!("Listening on {}...", addr);
    println!("Connect clients to this address");
    println!("Press Ctrl+C to stop\n");

    let state = Arc::new(RwLock::new(DeviceState::default()));

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                println!("[Server] New connection from {}", addr);
                let state_clone = state.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_client(stream, state_clone).await {
                        eprintln!("[Server] Error handling client: {}", e);
                    }
                });
            }
            Err(e) => {
                eprintln!("[Server] Accept error: {}", e);
            }
        }
    }
}
