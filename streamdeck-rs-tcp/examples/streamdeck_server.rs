//! Stream Deck Studio Server Emulator (Cora Protocol)
//!
//! This example implements a TCP server that emulates a Stream Deck Studio device.
//! It uses the Cora protocol for communication.

use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{RwLock, Mutex};
use streamdeck_rs_tcp::protocol::cora::{CoraMessage, CoraMessageFlags, CoraHidOp, CORA_MAGIC, CORA_HEADER_SIZE};

const DEFAULT_PORT: u16 = 5343;

#[derive(Debug, Clone)]
struct DeviceState {
    serial_number: String,
    firmware_version: String,
    brightness: u8,
    button_images: [Option<Vec<u8>>; 32],
    encoder_colors: [(u8, u8, u8); 2],
    device2_connected: bool, // Device 2 (Child Device) connection state
}

impl Default for DeviceState {
    fn default() -> Self {
        Self {
            serial_number: "EMULATOR123456".to_string(),
            firmware_version: "1.05.008".to_string(),
            brightness: 100,
            button_images: Default::default(),
            encoder_colors: [(0, 0, 0), (0, 0, 0)],
            device2_connected: false, // Device 2 starts disconnected
        }
    }
}

/// Send keep-alive packet to client
async fn send_keep_alive(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    next_message_id: &Arc<Mutex<u32>>,
    connection_no: &Arc<Mutex<u8>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut msg_id = next_message_id.lock().await;
    let conn_no = *connection_no.lock().await;
    
    let keep_alive_payload = {
        let mut payload = vec![0u8; 32];
        payload[0] = 0x01;
        payload[1] = 0x0a;
        payload[5] = conn_no;
        payload
    };

    let keep_alive_message = CoraMessage::new(
        CoraMessageFlags::None,
        CoraHidOp::SendReport,
        *msg_id,
        keep_alive_payload,
    );
    *msg_id += 1;

    let keep_alive_data = keep_alive_message.encode();
    writer.write_all(&keep_alive_data).await?;
    
    Ok(())
}

async fn handle_client(stream: TcpStream, state: Arc<RwLock<DeviceState>>) -> Result<(), Box<dyn std::error::Error>> {
    let (mut reader, mut writer) = stream.into_split();
    let mut receive_buffer = Vec::new();
    let mut _last_data_time = std::time::Instant::now();
    let next_message_id = Arc::new(Mutex::new(0u32));
    let connection_no = Arc::new(Mutex::new(0u8));

    println!("[Server] Client connected");

    // Wrap writer in Arc<Mutex<>> for sharing across tasks
    let writer_shared = Arc::new(Mutex::new(writer));
    
    // Send initial keep-alive packet (device sends this first)
    {
        let mut writer_guard = writer_shared.lock().await;
        send_keep_alive(&mut *writer_guard, &next_message_id, &connection_no).await?;
    }
    println!("[Server] Sent initial keep-alive");

    // Simulate Device 2 connection after a short delay (1 second)
    // In a real device, this would be triggered by actual hardware connection
    let writer_clone = writer_shared.clone();
    let next_message_id_clone = next_message_id.clone();
    let state_clone = state.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        
        // Connect Device 2
        {
            let mut device_state = state_clone.write().await;
            if !device_state.device2_connected {
                device_state.device2_connected = true;
                drop(device_state);
                
                // Send Device 2 plug event: connected
                let mut writer_guard = writer_clone.lock().await;
                if let Err(e) = send_device2_plug_event(&mut *writer_guard, &next_message_id_clone, true).await {
                    eprintln!("[Server] Error sending Device 2 plug event: {}", e);
                }
            }
        }
    });

    // Create interval for periodic keep-alive (4 seconds)
    let mut keep_alive_interval = tokio::time::interval(std::time::Duration::from_secs(4));
    keep_alive_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Skip the first tick (immediate execution) since we already sent initial keep-alive
    keep_alive_interval.tick().await;

    let mut read_chunk = vec![0u8; 4096];
    
    loop {
        tokio::select! {
            // Handle periodic keep-alive
            _ = keep_alive_interval.tick() => {
                let mut writer_guard = writer_shared.lock().await;
                if let Err(e) = send_keep_alive(&mut *writer_guard, &next_message_id, &connection_no).await {
                    eprintln!("[Server] Error sending keep-alive: {}", e);
                    break;
                }
            }

            // Read Cora message
            result = tokio::time::timeout(std::time::Duration::from_secs(10), reader.read(&mut read_chunk)) => {
                match result {
                    Ok(Ok(0)) => {
                        println!("[Server] Client disconnected");
                        break;
                    }
                    Ok(Ok(n)) => {
                        _last_data_time = std::time::Instant::now();
                        receive_buffer.extend_from_slice(&read_chunk[..n]);
                    }
                    Ok(Err(e)) => {
                        eprintln!("[Server] Read error: {}", e);
                        break;
                    }
                    Err(_) => {
                        // Read timeout - continue to check for keep-alive
                        continue;
                    }
                }
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
                    {
                        let mut writer_guard = writer_shared.lock().await;
                        if let Err(e) = handle_message(&message, &mut *writer_guard, &state, &next_message_id, &connection_no).await {
                            eprintln!("[Server] Error handling message: {} (continuing)", e);
                            // Continue processing even if one message fails
                        }
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

/// Extract report ID from GET_REPORT request payload
/// Returns None if the payload is not a valid GET_REPORT request
fn parse_get_report_request(payload: &[u8]) -> Option<u8> {
    if payload.len() >= 2 && payload[0] == 0x03 {
        Some(payload[1])
    } else {
        None
    }
}

/// Build GET_REPORT response payload in standard format
/// Format: [0x03, report_id, length_high, length_low, ...data...]
/// Length field is Big Endian (unlike other fields which are Little Endian)
fn build_get_report_response_payload(report_id: u8, data: &[u8]) -> Vec<u8> {
    let len = data.len().min(65535) as u16;
    let mut payload = Vec::with_capacity(4 + len as usize);
    
    payload.push(0x03);
    payload.push(report_id);
    payload.push(((len >> 8) & 0xff) as u8); // Big Endian high byte
    payload.push((len & 0xff) as u8); // Big Endian low byte
    payload.extend_from_slice(&data[..len as usize]);
    
    payload
}

/// Build Device 2 info response payload
/// Based on device2Info.ts: parseDevice2Info structure
/// Format: 128+ bytes with specific offsets for vendorId, productId, serial number, and TCP port
/// If connected is true, returns device info. If false, returns payload indicating disconnected (offset 4 != 0x02)
/// If for_plug_event is true, payload starts with 0x01 0x0b (for plug events), otherwise just the data (for GET_REPORT responses)
fn build_device2_info_payload(connected: bool, for_plug_event: bool) -> Vec<u8> {
    // Device 2 info is typically 128 bytes or more
    // Based on device2Info.ts structure:
    // - Offset 0-1: 0x01 0x0b (for plug events only)
    // - Offset 4: Device status (0x02 = connected/OK, != 0x02 = disconnected)
    // - Offset 26-27: vendorId (Little Endian, u16)
    // - Offset 28-29: productId (Little Endian, u16)
    // - Offset 94-125: Serial number (ASCII, up to 32 bytes)
    // - Offset 126-127: TCP port (Little Endian, u16)
    
    let mut payload = vec![0u8; 128];
    
    // For plug events, payload starts with 0x01 0x0b
    // For GET_REPORT responses, we don't include this prefix
    if for_plug_event {
        payload[0] = 0x01;
        payload[1] = 0x0b;
    }
    
    // Device status at offset 4 (0x02 = device connected, 0x00 = disconnected)
    if connected {
        payload[4] = 0x02;
        
        // Vendor ID at offset 26-27 (Little Endian)
        // Elgato Systems GmbH vendor ID: 0x0fd9
        let vendor_id: u16 = 0x0fd9;
        payload[26] = (vendor_id & 0xff) as u8;
        payload[27] = ((vendor_id >> 8) & 0xff) as u8;
        
        // Product ID at offset 28-29 (Little Endian)
        // Stream Deck Studio product ID (example: 0x0088)
        let product_id: u16 = 0x0088;
        payload[28] = (product_id & 0xff) as u8;
        payload[29] = ((product_id >> 8) & 0xff) as u8;
        
        // Serial number at offset 94-125 (ASCII, max 32 bytes)
        // Using a placeholder serial number
        let serial = b"DEVICE2_SERIAL_123456";
        let serial_len = serial.len().min(32);
        payload[94..94 + serial_len].copy_from_slice(&serial[..serial_len]);
        
        // TCP port at offset 126-127 (Little Endian)
        // Using default port 5343
        let tcp_port: u16 = 5343;
        payload[126] = (tcp_port & 0xff) as u8;
        payload[127] = ((tcp_port >> 8) & 0xff) as u8;
    } else {
        // Device disconnected - offset 4 remains 0x00
        payload[4] = 0x00;
    }
    
    payload
}

/// Send Device 2 plug event (0x01 0x0b) to client
/// This notifies the client when Device 2 connects or disconnects
async fn send_device2_plug_event(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    next_message_id: &Arc<Mutex<u32>>,
    connected: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut msg_id = next_message_id.lock().await;
    
    // Build payload with 0x01 0x0b prefix for plug events
    let payload = build_device2_info_payload(connected, true);
    
    let plug_event_message = CoraMessage::new(
        CoraMessageFlags::None,
        CoraHidOp::SendReport,
        *msg_id,
        payload,
    );
    *msg_id += 1;

    let event_data = plug_event_message.encode();
    writer.write_all(&event_data).await?;
    
    if connected {
        println!("[Server] Sent Device 2 plug event: connected");
    } else {
        println!("[Server] Sent Device 2 plug event: disconnected");
    }
    
    Ok(())
}

/// Get data for a specific report ID from device state
async fn get_report_data(state: &Arc<RwLock<DeviceState>>, report_id: u8) -> Option<Vec<u8>> {
    match report_id {
        0x83 => {
            // Firmware version
            Some(state.read().await.firmware_version.as_bytes().to_vec())
        }
        0x84 => {
            // Serial number
            Some(state.read().await.serial_number.as_bytes().to_vec())
        }
        0x1c => {
            // Device 2 (Child Device) information
            // Returns structured data as per device2Info.ts
            // For GET_REPORT responses, we don't include the 0x01 0x0b prefix
            Some(build_device2_info_payload(state.read().await.device2_connected, false))
        }
        0x8f | 0x87 | 0x1a => {
            // Unknown Report IDs - return empty data
            // 0x1a: Keep-alive related (already handled separately, but may come as GET_REPORT)
            // 0x8f, 0x87: Optional features or device-specific queries
            Some(Vec::new())
        }
        _ => {
            // Unknown Report ID - return None to indicate unsupported
            None
        }
    }
}

/// Send GET_REPORT response
async fn send_get_report_response(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    message_id: u32,
    report_id: u8,
    data: Vec<u8>,
) -> Result<(), Box<dyn std::error::Error>> {
    let response_payload = build_get_report_response_payload(report_id, &data);
    
    let response = CoraMessage::new(
        CoraMessageFlags::Result,
        CoraHidOp::GetReport,
        message_id,
        response_payload,
    );
    
    let response_data = response.encode();
    writer.write_all(&response_data).await?;
    
    Ok(())
}

async fn handle_message(
    message: &CoraMessage,
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    state: &Arc<RwLock<DeviceState>>,
    _next_message_id: &Arc<Mutex<u32>>,
    connection_no: &Arc<Mutex<u8>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Handle keep-alive ACK
    if message.flags == CoraMessageFlags::AckNak && message.payload.len() >= 3 && message.payload[0] == 0x03 && message.payload[1] == 0x1a {
        // Client responded to keep-alive
        let mut conn_no = connection_no.lock().await;
        *conn_no = message.payload[2];
        return Ok(());
    }

    // Handle GET_REPORT requests regardless of HID operation (0x03 0x83, 0x03 0x84, etc.)
    // Some clients may send GET_REPORT requests with different HID operations
    if let Some(report_id) = parse_get_report_request(&message.payload) {
        if let Some(data) = get_report_data(state, report_id).await {
            let report_name = match report_id {
                0x83 => "Firmware version",
                0x84 => "Serial number",
                0x1c => "Device 2 (Child Device) info",
                0x8f => "Report ID 0x8f",
                0x87 => "Report ID 0x87",
                0x1a => "Report ID 0x1a (Keep-alive related)",
                _ => "Unknown Report ID",
            };
            
            let data_len = data.len();
            println!("[Server] {} request (Report ID: 0x{:02x}, HID op: {:?})", 
                report_name, report_id, message.hid_op);
            
            send_get_report_response(writer, message.message_id, report_id, data).await?;
            
            if report_id == 0x83 {
                println!("[Server] Sent firmware version: {}", 
                    state.read().await.firmware_version);
            } else if report_id == 0x84 {
                println!("[Server] Sent serial: {}", 
                    state.read().await.serial_number);
            } else {
                println!("[Server] Sent response for Report ID 0x{:02x} ({} bytes)", 
                    report_id, data_len);
            }
            
            return Ok(());
        } else {
            println!("[Server] Unsupported GET_REPORT request: Report ID 0x{:02x}", report_id);
        }
    }

    // Handle commands based on HID operation
    match message.hid_op {
        CoraHidOp::GetReport => {
            // GET_REPORT requests are already handled above (before the match statement)
            // This is a fallback for any other GET_REPORT requests
            println!("[Server] GET_REPORT request (already handled above or unknown): {:?}", message.payload);
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

                    _ => {
                        println!("[Server] Unknown SEND_REPORT command: 0x{:02x} 0x{:02x}", 
                            message.payload.get(0).copied().unwrap_or(0),
                            message.payload.get(1).copied().unwrap_or(0));
                    }
                }
            } else {
                println!("[Server] SEND_REPORT command too short: {} bytes", message.payload.len());
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
