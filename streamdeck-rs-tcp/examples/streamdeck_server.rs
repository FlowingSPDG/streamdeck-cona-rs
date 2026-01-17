//! Stream Deck Studio Server Emulator
//!
//! This example implements a TCP server that emulates a Stream Deck Studio device.
//! It can be used for testing clients or as a reference implementation.

use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;

const PACKET_SIZE: usize = 1024;
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
    let mut buffer = [0u8; PACKET_SIZE];
    let mut _last_data_time = std::time::Instant::now();

    println!("[Server] Client connected");

    loop {
        // Read packet
        match tokio::time::timeout(std::time::Duration::from_secs(10), reader.read_exact(&mut buffer)).await {
            Ok(Ok(_)) => {
                _last_data_time = std::time::Instant::now();
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

        // Parse command
        match (buffer[0], buffer[1]) {
            // Serial number request
            (0x03, 0x84) => {
                println!("[Server] Serial number request");
                let mut response = [0u8; PACKET_SIZE];
                response[0] = 0x03;
                response[1] = 0x84;
                let serial = state.read().await.serial_number.clone();
                let len = serial.len().min(65535) as u16;
                response[2] = ((len >> 8) & 0xff) as u8; // Big Endian
                response[3] = (len & 0xff) as u8;
                response[4..4 + len as usize].copy_from_slice(serial.as_bytes());
                writer.write_all(&response).await?;
                println!("[Server] Sent serial: {}", serial);
            }

            // Brightness set
            (0x03, 0x08) => {
                let brightness = buffer[2];
                state.write().await.brightness = brightness;
                println!("[Server] Brightness set to {}%", brightness);
            }

            // Button image
            (0x02, 0x07) => {
                let button_index = buffer[2] as usize;
                if button_index < 32 {
                    let is_last_page = buffer[3] == 0x01;
                    let data_len = u16::from_le_bytes([buffer[4], buffer[5]]) as usize;
                    let page_num = u16::from_le_bytes([buffer[6], buffer[7]]);
                    
                    if page_num == 0 {
                        // First page - create new image data
                        state.write().await.button_images[button_index] = Some(buffer[8..8 + data_len].to_vec());
                    } else if let Some(ref mut img_data) = state.write().await.button_images[button_index] {
                        // Subsequent page - append data
                        img_data.extend_from_slice(&buffer[8..8 + data_len]);
                    }

                    if is_last_page {
                        println!("[Server] Button {} image set ({} bytes)", button_index, 
                            state.read().await.button_images[button_index].as_ref().map(|v| v.len()).unwrap_or(0));
                    }
                }
            }

            // Encoder color
            (0x02, 0x10) => {
                let encoder_index = buffer[2] as usize;
                if encoder_index < 2 {
                    let r = buffer[3];
                    let g = buffer[4];
                    let b = buffer[5];
                    state.write().await.encoder_colors[encoder_index] = (r, g, b);
                    println!("[Server] Encoder {} color set to RGB({},{},{})", encoder_index, r, g, b);
                }
            }

            // Encoder ring
            (0x02, 0x0f) => {
                let encoder_index = buffer[2] as usize;
                if encoder_index < 2 {
                    let mut colors = Vec::new();
                    for i in 0..24 {
                        let offset = 3 + i * 3;
                        if offset + 2 < PACKET_SIZE {
                            colors.push((buffer[offset], buffer[offset + 1], buffer[offset + 2]));
                        }
                    }
                    println!("[Server] Encoder {} ring set with {} colors", encoder_index, colors.len());
                }
            }

            // Display area image
            (0x02, 0x0c) => {
                let x = u16::from_le_bytes([buffer[2], buffer[3]]);
                let y = u16::from_le_bytes([buffer[4], buffer[5]]);
                let width = u16::from_le_bytes([buffer[6], buffer[7]]);
                let height = u16::from_le_bytes([buffer[8], buffer[9]]);
                let is_last_page = buffer[10] == 0x01;
                let page_num = u16::from_le_bytes([buffer[11], buffer[12]]);
                let _data_len = u16::from_le_bytes([buffer[13], buffer[14]]) as usize;

                if is_last_page {
                    println!("[Server] Display area image at ({},{}) size {}x{} (page {})", 
                        x, y, width, height, page_num);
                }
            }

            // Keep-alive response
            (0x03, 0x1a) => {
                // Client responded to keep-alive, do nothing
            }

            // Reset
            (0x03, 0x02) => {
                println!("[Server] Reset command received");
                *state.write().await = DeviceState::default();
            }

            // Event from client (shouldn't happen, but handle gracefully)
            (0x01, _) => {
                println!("[Server] Received event packet (unexpected)");
            }

            _ => {
                println!("[Server] Unknown command: 0x{:02x} 0x{:02x}", buffer[0], buffer[1]);
            }
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
