//! Stream Deck Studio Server Emulator (Cora Protocol)
//!
//! This example implements a TCP server that emulates a Stream Deck Studio device.
//! It uses the Cora protocol for communication.

use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use streamdeck_rs_tcp::protocol::cora::{CoraMessage, CoraMessageFlags, CoraHidOp, CORA_MAGIC, CORA_HEADER_SIZE};

const DEFAULT_PORT: u16 = 5343;

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
    let mut receive_buffer = Vec::new();
    let mut _last_data_time = std::time::Instant::now();
    let mut next_message_id = 0u32;
    let mut connection_no = 0u8;

    println!("[Server] Client connected");

    // Send initial keep-alive packet (device sends this first)
    let keep_alive_payload = {
        let mut payload = vec![0u8; 32];
        payload[0] = 0x01;
        payload[1] = 0x0a;
        payload[5] = connection_no;
        payload
    };

    let keep_alive_message = CoraMessage::new(
        CoraMessageFlags::None,
        CoraHidOp::SendReport,
        next_message_id,
        keep_alive_payload,
    );
    next_message_id += 1;

    let keep_alive_data = keep_alive_message.encode();
    writer.write_all(&keep_alive_data).await?;
    println!("[Server] Sent initial keep-alive");

    loop {
        // Read Cora message
        let mut chunk = vec![0u8; 4096];
        match tokio::time::timeout(std::time::Duration::from_secs(10), reader.read(&mut chunk)).await {
            Ok(Ok(0)) => {
                println!("[Server] Client disconnected");
                break;
            }
            Ok(Ok(n)) => {
                _last_data_time = std::time::Instant::now();
                receive_buffer.extend_from_slice(&chunk[..n]);
            }
            Ok(Err(e)) => {
                eprintln!("[Server] Read error: {}", e);
                break;
            }
            Err(_) => {
                eprintln!("[Server] Read timeout");
                break;
            }
        }

        // Check if timeout exceeded
        if _last_data_time.elapsed() > std::time::Duration::from_secs(5) {
            eprintln!("[Server] Client timeout");
            break;
        }

        // Process complete messages from buffer
        while receive_buffer.len() >= CORA_HEADER_SIZE {
            // Find magic bytes
            let magic_pos = receive_buffer.windows(4).position(|w| w == &CORA_MAGIC);
            
            if let Some(pos) = magic_pos {
                if pos > 0 {
                    // Remove data before magic
                    receive_buffer.drain(..pos);
                }
            } else {
                // No magic found, keep last 3 bytes
                if receive_buffer.len() > 3 {
                    let keep = receive_buffer[receive_buffer.len() - 3..].to_vec();
                    receive_buffer.clear();
                    receive_buffer.extend_from_slice(&keep);
                }
                break;
            }

            // Check if we have full header
            if receive_buffer.len() < CORA_HEADER_SIZE {
                break;
            }

            // Read payload length from header
            let payload_length = u32::from_le_bytes([
                receive_buffer[12], receive_buffer[13], receive_buffer[14], receive_buffer[15]
            ]) as usize;

            // Check if we have full message
            if receive_buffer.len() < CORA_HEADER_SIZE + payload_length {
                break;
            }

            // Decode message
            match CoraMessage::decode(&receive_buffer[..CORA_HEADER_SIZE + payload_length]) {
                Ok(message) => {
                    // Remove processed message from buffer
                    receive_buffer.drain(..CORA_HEADER_SIZE + payload_length);

                    // Handle message
                    if let Err(e) = handle_message(&message, &mut writer, &state, &mut next_message_id, &mut connection_no).await {
                        eprintln!("[Server] Error handling message: {}", e);
                    }
                }
                Err(e) => {
                    eprintln!("[Server] Failed to decode message: {}", e);
                    // Skip this message by removing magic bytes and continue
                    if let Some(pos) = receive_buffer[1..].windows(4).position(|w| w == &CORA_MAGIC) {
                        receive_buffer.drain(..pos + 1);
                    } else {
                        receive_buffer.clear();
                    }
                }
            }
        }
    }

    println!("[Server] Client disconnected");
    Ok(())
}

async fn handle_message(
    message: &CoraMessage,
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    state: &Arc<RwLock<DeviceState>>,
    next_message_id: &mut u32,
    connection_no: &mut u8,
) -> Result<(), Box<dyn std::error::Error>> {
    // Handle keep-alive ACK
    if message.flags == CoraMessageFlags::AckNak && message.payload.len() >= 3 && message.payload[0] == 0x03 && message.payload[1] == 0x1a {
        // Client responded to keep-alive
        *connection_no = message.payload[2];
        return Ok(());
    }

    // Handle commands based on HID operation
    match message.hid_op {
        CoraHidOp::GetReport => {
            // Handle GET_REPORT requests
            if message.payload.len() >= 2 && message.payload[0] == 0x03 && message.payload[1] == 0x84 {
                // Serial number request
                println!("[Server] Serial number request");
                let serial = state.read().await.serial_number.clone();
                let len = serial.len().min(65535) as u16;

                let mut response_payload = vec![0u8; 4 + len as usize];
                response_payload[0] = 0x03;
                response_payload[1] = 0x84;
                response_payload[2] = ((len >> 8) & 0xff) as u8; // Big Endian
                response_payload[3] = (len & 0xff) as u8;
                response_payload[4..4 + len as usize].copy_from_slice(serial.as_bytes());

                let response = CoraMessage::new(
                    CoraMessageFlags::Result,
                    CoraHidOp::GetReport,
                    message.message_id,
                    response_payload,
                );

                let response_data = response.encode();
                writer.write_all(&response_data).await?;
                println!("[Server] Sent serial: {}", serial);
            }
        }

        CoraHidOp::SendReport => {
            // Handle SEND_REPORT commands
            if message.payload.len() >= 2 {
                match (message.payload[0], message.payload[1]) {
                    // Brightness set (0x03 0x08)
                    (0x03, 0x08) if message.payload.len() >= 3 => {
                        let brightness = message.payload[2];
                        state.write().await.brightness = brightness;
                        println!("[Server] Brightness set to {}%", brightness);
                    }

                    // Reset (0x03 0x02)
                    (0x03, 0x02) => {
                        println!("[Server] Reset command received");
                        *state.write().await = DeviceState::default();
                    }

                    _ => {}
                }
            }
        }

        CoraHidOp::Write => {
            // Handle WRITE commands (images, encoder colors, etc.)
            if message.payload.len() >= 2 {
                match (message.payload[0], message.payload[1]) {
                    // Button image (0x02 0x07)
                    (0x02, 0x07) if message.payload.len() >= 8 => {
                        let button_index = message.payload[2] as usize;
                        if button_index < 32 {
                            let is_last_page = message.payload[3] == 0x01;
                            let data_len = u16::from_le_bytes([message.payload[4], message.payload[5]]) as usize;
                            let page_num = u16::from_le_bytes([message.payload[6], message.payload[7]]);

                            if page_num == 0 {
                                // First page - create new image data
                                let data_start = 8;
                                let data_end = (data_start + data_len).min(message.payload.len());
                                state.write().await.button_images[button_index] = Some(message.payload[data_start..data_end].to_vec());
                            } else if let Some(ref mut img_data) = state.write().await.button_images[button_index] {
                                // Subsequent page - append data
                                let data_start = 8;
                                let data_end = (data_start + data_len).min(message.payload.len());
                                img_data.extend_from_slice(&message.payload[data_start..data_end]);
                            }

                            if is_last_page {
                                println!("[Server] Button {} image set ({} bytes)", button_index,
                                    state.read().await.button_images[button_index].as_ref().map(|v| v.len()).unwrap_or(0));
                            }
                        }
                    }

                    // Encoder color (0x02 0x10)
                    (0x02, 0x10) if message.payload.len() >= 6 => {
                        let encoder_index = message.payload[2] as usize;
                        if encoder_index < 2 {
                            let r = message.payload[3];
                            let g = message.payload[4];
                            let b = message.payload[5];
                            state.write().await.encoder_colors[encoder_index] = (r, g, b);
                            println!("[Server] Encoder {} color set to RGB({},{},{})", encoder_index, r, g, b);
                        }
                    }

                    // Encoder ring (0x02 0x0f)
                    (0x02, 0x0f) if message.payload.len() >= 3 => {
                        let encoder_index = message.payload[2] as usize;
                        if encoder_index < 2 {
                            let mut colors = Vec::new();
                            for i in 0..24 {
                                let offset = 3 + i * 3;
                                if offset + 2 < message.payload.len() {
                                    colors.push((message.payload[offset], message.payload[offset + 1], message.payload[offset + 2]));
                                }
                            }
                            println!("[Server] Encoder {} ring set with {} colors", encoder_index, colors.len());
                        }
                    }

                    // Display area image (0x02 0x0c)
                    (0x02, 0x0c) if message.payload.len() >= 16 => {
                        let x = u16::from_le_bytes([message.payload[2], message.payload[3]]);
                        let y = u16::from_le_bytes([message.payload[4], message.payload[5]]);
                        let width = u16::from_le_bytes([message.payload[6], message.payload[7]]);
                        let height = u16::from_le_bytes([message.payload[8], message.payload[9]]);
                        let is_last_page = message.payload[10] == 0x01;
                        let page_num = u16::from_le_bytes([message.payload[11], message.payload[12]]);

                        if is_last_page {
                            println!("[Server] Display area image at ({},{}) size {}x{} (page {})",
                                x, y, width, height, page_num);
                        }
                    }

                    _ => {
                        println!("[Server] Unknown command: 0x{:02x} 0x{:02x}", 
                            message.payload.get(0).copied().unwrap_or(0),
                            message.payload.get(1).copied().unwrap_or(0));
                    }
                }
            }
        }
    }

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

    println!("Stream Deck Studio Emulator (Cora Protocol)");
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
