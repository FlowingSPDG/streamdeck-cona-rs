//! Stream Deck NetworkDock Server Emulator (Cora Protocol)
//!
//! This is a CUI tool that implements a TCP server that emulates a Stream Deck NetworkDock device.
//! It uses the Cora protocol for communication.
//!
//! Usage: networkdock-server [PORT]

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{RwLock, Mutex};
use streamdeck_cona_rs::protocol::{CoraMessage, CoraMessageFlags, CoraHidOp, CORA_MAGIC, CORA_HEADER_SIZE};

const DEFAULT_PORT: u16 = 5343;

/// Log levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum LogLevel {
    Error = 0,
    Warn = 1,
    Info = 2,
    Debug = 3,
    Trace = 4,
}

impl LogLevel {
    fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "error" => LogLevel::Error,
            "warn" => LogLevel::Warn,
            "info" => LogLevel::Info,
            "debug" => LogLevel::Debug,
            "trace" => LogLevel::Trace,
            _ => LogLevel::Info,
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Error => "ERROR",
            LogLevel::Warn => "WARN ",
            LogLevel::Info => "INFO ",
            LogLevel::Debug => "DEBUG",
            LogLevel::Trace => "TRACE",
        }
    }
}

/// Global log level (defaults to INFO, can be set via RUST_LOG environment variable)
fn get_log_level() -> LogLevel {
    std::env::var("RUST_LOG")
        .unwrap_or_else(|_| "info".to_string())
        .to_lowercase()
        .split(',')
        .next()
        .map(|s| {
            let level = s.trim();
            if level.starts_with("streamdeck_server=") {
                LogLevel::from_str(level.split('=').nth(1).unwrap_or("info"))
            } else {
                LogLevel::from_str(level)
            }
        })
        .unwrap_or(LogLevel::Info)
}

/// Format current time as [HH:MM:SS]
fn format_time() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = now.as_secs();
    let hours = (total_secs / 3600) % 24;
    let minutes = (total_secs / 60) % 60;
    let seconds = total_secs % 60;
    format!("[{:02}:{:02}:{:02}]", hours, minutes, seconds)
}

/// Log message with level and timestamp
fn log(level: LogLevel, message: &str) {
    let max_level = get_log_level();
    if level <= max_level {
        println!("{} [{}] {}", format_time(), level.as_str(), message);
    }
}

/// Log error message
fn log_error(message: &str) {
    eprintln!("{} [ERROR] {}", format_time(), message);
}

/// Throttled logger - limits log frequency
struct ThrottledLogger {
    last_log_time: std::time::Instant,
    interval: std::time::Duration,
    skip_count: std::sync::atomic::AtomicU64,
}

impl ThrottledLogger {
    fn new(interval_secs: u64) -> Self {
        Self {
            last_log_time: std::time::Instant::now() - std::time::Duration::from_secs(interval_secs),
            interval: std::time::Duration::from_secs(interval_secs),
            skip_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    fn should_log(&mut self) -> bool {
        let now = std::time::Instant::now();
        if now.duration_since(self.last_log_time) >= self.interval {
            let skipped = self.skip_count.swap(0, std::sync::atomic::Ordering::Relaxed);
            if skipped > 0 {
                eprintln!("{} [INFO ] ... ({} messages skipped)", format_time(), skipped);
            }
            self.last_log_time = now;
            true
        } else {
            self.skip_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            false
        }
    }

    fn log(&mut self, level: LogLevel, message: &str) {
        if self.should_log() {
            log(level, message);
        }
    }
}

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

/// Client connection information
struct ClientConnection {
    writer: Arc<Mutex<tokio::net::tcp::OwnedWriteHalf>>,
    next_message_id: Arc<Mutex<u32>>,
}

/// Send button event to client
/// Payload: [0x01, 0x00, 0x00, 0x00, ...button_states...] (36 bytes total: 4 header + 32 button states)
async fn send_button_event(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    next_message_id: &Arc<Mutex<u32>>,
    button_index: u8,
    pressed: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if button_index >= 32 {
        return Err("Button index out of range (0-31)".into());
    }

    let mut msg_id = next_message_id.lock().await;
    
    let mut payload = vec![0u8; 36];
    payload[0] = 0x01;
    payload[1] = 0x00;
    payload[2] = 0x00;
    payload[3] = 0x00;
    // Button states start at offset 4 (32 buttons)
    payload[4 + button_index as usize] = if pressed { 0x01 } else { 0x00 };

    let event_message = CoraMessage::new(
        CoraMessageFlags::None,
        CoraHidOp::SendReport,
        *msg_id,
        payload,
    );
    *msg_id += 1;

    let event_data = event_message.encode();
    writer.write_all(&event_data).await?;
    
    Ok(())
}

/// Send encoder rotate event to client
/// Payload: [0x01, 0x03, 0x00, 0x00, 0x01, rot0, rot1]
async fn send_encoder_rotate_event(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    next_message_id: &Arc<Mutex<u32>>,
    encoder_index: u8,
    delta: i8,
) -> Result<(), Box<dyn std::error::Error>> {
    if encoder_index >= 2 {
        return Err("Encoder index out of range (0-1)".into());
    }

    let mut msg_id = next_message_id.lock().await;
    
    let mut payload = vec![0u8; 7];
    payload[0] = 0x01;
    payload[1] = 0x03;
    payload[2] = 0x00;
    payload[3] = 0x00;
    payload[4] = 0x01; // Subtype: rotation
    // Encoder rotation values start at offset 5
    payload[5] = if encoder_index == 0 { delta as u8 } else { 0 };
    payload[6] = if encoder_index == 1 { delta as u8 } else { 0 };

    let event_message = CoraMessage::new(
        CoraMessageFlags::None,
        CoraHidOp::SendReport,
        *msg_id,
        payload,
    );
    *msg_id += 1;

    let event_data = event_message.encode();
    writer.write_all(&event_data).await?;
    
    Ok(())
}

/// Send encoder press event to client
/// Payload: [0x01, 0x03, 0x00, 0x00, 0x00, press0, press1]
async fn send_encoder_press_event(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    next_message_id: &Arc<Mutex<u32>>,
    encoder_index: u8,
    pressed: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if encoder_index >= 2 {
        return Err("Encoder index out of range (0-1)".into());
    }

    let mut msg_id = next_message_id.lock().await;
    
    let mut payload = vec![0u8; 7];
    payload[0] = 0x01;
    payload[1] = 0x03;
    payload[2] = 0x00;
    payload[3] = 0x00;
    payload[4] = 0x00; // Subtype: press
    // Encoder press states start at offset 5
    payload[5] = if encoder_index == 0 && pressed { 0x01 } else { 0x00 };
    payload[6] = if encoder_index == 1 && pressed { 0x01 } else { 0x00 };

    let event_message = CoraMessage::new(
        CoraMessageFlags::None,
        CoraHidOp::SendReport,
        *msg_id,
        payload,
    );
    *msg_id += 1;

    let event_data = event_message.encode();
    writer.write_all(&event_data).await?;
    
    Ok(())
}

/// Send touch event to client
/// Payload for tap/press: [0x01, 0x02, 0x00, 0x00, 0x01/0x02, 0x00, x_low, x_high, y_low, y_high]
/// Payload for swipe: [0x01, 0x02, 0x00, 0x00, 0x03, 0x00, x0_low, x0_high, y0_low, y0_high, x1_low, x1_high, y1_low, y1_high]
async fn send_touch_event(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    next_message_id: &Arc<Mutex<u32>>,
    touch_type: u8, // 0x01 = tap, 0x02 = press, 0x03 = swipe
    x: u16,
    y: u16,
    end_x: Option<u16>,
    end_y: Option<u16>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut msg_id = next_message_id.lock().await;
    
    let payload = if touch_type == 0x03 && end_x.is_some() && end_y.is_some() {
        // Swipe event
        let ex = end_x.unwrap();
        let ey = end_y.unwrap();
        vec![
            0x01, 0x02, 0x00, 0x00, 0x03, 0x00,
            (x & 0xff) as u8, ((x >> 8) & 0xff) as u8,
            (y & 0xff) as u8, ((y >> 8) & 0xff) as u8,
            (ex & 0xff) as u8, ((ex >> 8) & 0xff) as u8,
            (ey & 0xff) as u8, ((ey >> 8) & 0xff) as u8,
        ]
    } else {
        // Tap or press event
        vec![
            0x01, 0x02, 0x00, 0x00, touch_type, 0x00,
            (x & 0xff) as u8, ((x >> 8) & 0xff) as u8,
            (y & 0xff) as u8, ((y >> 8) & 0xff) as u8,
        ]
    };

    let event_message = CoraMessage::new(
        CoraMessageFlags::None,
        CoraHidOp::SendReport,
        *msg_id,
        payload,
    );
    *msg_id += 1;

    let event_data = event_message.encode();
    writer.write_all(&event_data).await?;
    
    Ok(())
}

/// Broadcast event to all connected clients
async fn broadcast_button_event(
    clients: &Arc<Mutex<Vec<ClientConnection>>>,
    button_index: u8,
    pressed: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut clients_guard = clients.lock().await;
    let mut disconnected = Vec::new();
    
    for (i, client) in clients_guard.iter().enumerate() {
        let mut writer = client.writer.lock().await;
        if let Err(_) = send_button_event(&mut *writer, &client.next_message_id, button_index, pressed).await {
            disconnected.push(i);
        }
    }
    
    // Remove disconnected clients (in reverse order to maintain indices)
    for &i in disconnected.iter().rev() {
        clients_guard.remove(i);
    }
    
    Ok(())
}

async fn broadcast_encoder_rotate_event(
    clients: &Arc<Mutex<Vec<ClientConnection>>>,
    encoder_index: u8,
    delta: i8,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut clients_guard = clients.lock().await;
    let mut disconnected = Vec::new();
    
    for (i, client) in clients_guard.iter().enumerate() {
        let mut writer = client.writer.lock().await;
        if let Err(_) = send_encoder_rotate_event(&mut *writer, &client.next_message_id, encoder_index, delta).await {
            disconnected.push(i);
        }
    }
    
    for &i in disconnected.iter().rev() {
        clients_guard.remove(i);
    }
    
    Ok(())
}

async fn broadcast_encoder_press_event(
    clients: &Arc<Mutex<Vec<ClientConnection>>>,
    encoder_index: u8,
    pressed: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut clients_guard = clients.lock().await;
    let mut disconnected = Vec::new();
    
    for (i, client) in clients_guard.iter().enumerate() {
        let mut writer = client.writer.lock().await;
        if let Err(_) = send_encoder_press_event(&mut *writer, &client.next_message_id, encoder_index, pressed).await {
            disconnected.push(i);
        }
    }
    
    for &i in disconnected.iter().rev() {
        clients_guard.remove(i);
    }
    
    Ok(())
}

async fn broadcast_touch_event(
    clients: &Arc<Mutex<Vec<ClientConnection>>>,
    touch_type: u8,
    x: u16,
    y: u16,
    end_x: Option<u16>,
    end_y: Option<u16>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut clients_guard = clients.lock().await;
    let mut disconnected = Vec::new();
    
    for (i, client) in clients_guard.iter().enumerate() {
        let mut writer = client.writer.lock().await;
        if let Err(_) = send_touch_event(&mut *writer, &client.next_message_id, touch_type, x, y, end_x, end_y).await {
            disconnected.push(i);
        }
    }
    
    for &i in disconnected.iter().rev() {
        clients_guard.remove(i);
    }
    
    Ok(())
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

async fn handle_client(
    stream: TcpStream,
    state: Arc<RwLock<DeviceState>>,
    clients: Arc<Mutex<Vec<ClientConnection>>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut reader, writer) = stream.into_split();
    let mut receive_buffer = Vec::new();
    let mut _last_data_time = std::time::Instant::now();
    let next_message_id = Arc::new(Mutex::new(0u32));
    let connection_no = Arc::new(Mutex::new(0u8));
    
    // Throttled logger for frequent logs (5 seconds interval)
    let mut throttled_logger = ThrottledLogger::new(5);

    log(LogLevel::Info, &format!("[Server] Client connected"));

    // Wrap writer in Arc<Mutex<>> for sharing across tasks
    let writer_shared = Arc::new(Mutex::new(writer));
    
    // Send initial keep-alive packet (device sends this first)
    {
        let mut writer_guard = writer_shared.lock().await;
        send_keep_alive(&mut *writer_guard, &next_message_id, &connection_no).await?;
    }
    
    // Register this client after initial keep-alive is sent
    let client_conn = ClientConnection {
        writer: writer_shared.clone(),
        next_message_id: next_message_id.clone(),
    };
    {
        let mut clients_guard = clients.lock().await;
        clients_guard.push(client_conn);
    }

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
                    log_error(&format!("[Server] Error sending Device 2 plug event: {}", e));
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
                    log_error(&format!("[Server] Error sending keep-alive: {}", e));
                    break;
                }
            }

            // Read Cora message
            result = tokio::time::timeout(std::time::Duration::from_secs(10), reader.read(&mut read_chunk)) => {
                match result {
                    Ok(Ok(0)) => {
                        log(LogLevel::Info, &format!("[Server] Client disconnected"));
                        break;
                    }
                    Ok(Ok(n)) => {
                        _last_data_time = std::time::Instant::now();
                        receive_buffer.extend_from_slice(&read_chunk[..n]);
                    }
                    Ok(Err(e)) => {
                        log_error(&format!("[Server] Read error: {}", e));
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
            log_error("[Server] Client timeout");
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
                        if let Err(e) = handle_message(&message, &mut *writer_guard, &state, &next_message_id, &connection_no, &mut throttled_logger).await {
                            log_error(&format!("[Server] Error handling message: {} (continuing)", e));
                            // Continue processing even if one message fails
                        }
                    }
                }
                Err(e) => {
                    // Use throttled logger for decode errors (can happen frequently)
                    throttled_logger.log(LogLevel::Debug, &format!("[Server] Failed to decode message: {}", e));
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

    log(LogLevel::Info, "[Server] Client disconnected");
    
    // Unregister this client
    {
        let mut clients_guard = clients.lock().await;
        clients_guard.retain(|c| !Arc::ptr_eq(&c.writer, &writer_shared));
    }
    
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
        // Stream Deck NetworkDock product ID
        let product_id: u16 = 0x0089; // NetworkDock product ID
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
    throttled_logger: &mut ThrottledLogger,
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
            // Use throttled logger for GET_REPORT requests (can be very frequent)
            throttled_logger.log(LogLevel::Debug, &format!("[Server] GET_REPORT request (Report ID: 0x{:02x})", report_id));
            send_get_report_response(writer, message.message_id, report_id, data).await?;
            return Ok(());
        }
    }

    // Handle commands based on HID operation
    match message.hid_op {
        CoraHidOp::GetReport => {
            // GET_REPORT requests are already handled above (before the match statement)
            // This is a fallback for any other GET_REPORT requests
        }

        CoraHidOp::SendReport => {
            // Handle SEND_REPORT commands
            if message.payload.len() >= 2 {
                match (message.payload[0], message.payload[1]) {
                    // Brightness set (0x03 0x08)
                    (0x03, 0x08) if message.payload.len() >= 3 => {
                        let brightness = message.payload[2];
                        state.write().await.brightness = brightness;
                    }

                    // Reset (0x03 0x02)
                    (0x03, 0x02) => {
                        *state.write().await = DeviceState::default();
                    }

                    _ => {
                        // Unknown SEND_REPORT command - silently ignore
                    }
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

                            // Image data received and stored
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
                        }
                    }

                    // Encoder ring (0x02 0x0f)
                    (0x02, 0x0f) if message.payload.len() >= 3 => {
                        let encoder_index = message.payload[2] as usize;
                        if encoder_index < 2 {
                            // Encoder ring colors processed
                        }
                    }

                    // Display area image (0x02 0x0c)
                    (0x02, 0x0c) if message.payload.len() >= 16 => {
                        // Display area image data processed
                    }

                    _ => {
                        // Unknown command - silently ignore
                    }
                }
            }
        }
    }

    Ok(())
}

/// Parse command from input line
fn parse_command(line: &str) -> Result<Command, String> {
    let parts: Vec<&str> = line.trim().split_whitespace().collect();
    if parts.is_empty() {
        return Err("Empty command".to_string());
    }

    match parts[0].to_lowercase().as_str() {
        "button" => {
            if parts.len() < 3 {
                return Err("Usage: button press <index> | button release <index>".to_string());
            }
            let action = parts[1].to_lowercase();
            let index: u8 = parts[2].parse().map_err(|_| "Invalid button index (0-31)".to_string())?;
            if index >= 32 {
                return Err("Button index out of range (0-31)".to_string());
            }
            match action.as_str() {
                "press" => Ok(Command::ButtonPress(index)),
                "release" => Ok(Command::ButtonRelease(index)),
                _ => Err("Invalid button action: use 'press' or 'release'".to_string()),
            }
        }
        "encoder" => {
            if parts.len() < 3 {
                return Err("Usage: encoder rotate/press/release <index> [delta]".to_string());
            }
            let action = parts[1].to_lowercase();
            let index: u8 = parts[2].parse().map_err(|_| "Invalid encoder index (0-1)".to_string())?;
            if index >= 2 {
                return Err("Encoder index out of range (0-1)".to_string());
            }
            match action.as_str() {
                "rotate" => {
                    if parts.len() < 4 {
                        return Err("Usage: encoder rotate <index> <delta>".to_string());
                    }
                    let delta: i8 = parts[3].parse().map_err(|_| "Invalid delta (-128 to 127)".to_string())?;
                    Ok(Command::EncoderRotate(index, delta))
                }
                "press" => Ok(Command::EncoderPress(index)),
                "release" => Ok(Command::EncoderRelease(index)),
                _ => Err("Invalid encoder action: use 'rotate', 'press', or 'release'".to_string()),
            }
        }
        "touch" => {
            if parts.len() < 4 {
                return Err("Usage: touch tap/press <x> <y> | touch swipe <x1> <y1> <x2> <y2>".to_string());
            }
            let action = parts[1].to_lowercase();
            match action.as_str() {
                "tap" | "press" => {
                    if parts.len() < 4 {
                        return Err("Usage: touch tap/press <x> <y>".to_string());
                    }
                    let x: u16 = parts[2].parse().map_err(|_| "Invalid x coordinate".to_string())?;
                    let y: u16 = parts[3].parse().map_err(|_| "Invalid y coordinate".to_string())?;
                    let touch_type = if action == "tap" { 0x01 } else { 0x02 };
                    Ok(Command::Touch(touch_type, x, y, None, None))
                }
                "swipe" => {
                    if parts.len() < 6 {
                        return Err("Usage: touch swipe <x1> <y1> <x2> <y2>".to_string());
                    }
                    let x1: u16 = parts[2].parse().map_err(|_| "Invalid x1 coordinate".to_string())?;
                    let y1: u16 = parts[3].parse().map_err(|_| "Invalid y1 coordinate".to_string())?;
                    let x2: u16 = parts[4].parse().map_err(|_| "Invalid x2 coordinate".to_string())?;
                    let y2: u16 = parts[5].parse().map_err(|_| "Invalid y2 coordinate".to_string())?;
                    Ok(Command::Touch(0x03, x1, y1, Some(x2), Some(y2)))
                }
                _ => Err("Invalid touch action: use 'tap', 'press', or 'swipe'".to_string()),
            }
        }
        "help" => Ok(Command::Help),
        "quit" | "exit" => Ok(Command::Quit),
        _ => Err(format!("Unknown command: {}. Type 'help' for available commands", parts[0])),
    }
}

/// Command types
enum Command {
    ButtonPress(u8),
    ButtonRelease(u8),
    EncoderRotate(u8, i8),
    EncoderPress(u8),
    EncoderRelease(u8),
    Touch(u8, u16, u16, Option<u16>, Option<u16>),
    Help,
    Quit,
}

/// Print help message
fn print_help() {
    println!("\nAvailable commands:");
    println!("  button press <index>     - Press button (index: 0-31)");
    println!("  button release <index>   - Release button (index: 0-31)");
    println!("  encoder rotate <index> <delta> - Rotate encoder (index: 0-1, delta: -128 to 127)");
    println!("  encoder press <index>    - Press encoder (index: 0-1)");
    println!("  encoder release <index>  - Release encoder (index: 0-1)");
    println!("  touch tap <x> <y>        - Touch tap at coordinates");
    println!("  touch press <x> <y>      - Touch press at coordinates");
    println!("  touch swipe <x1> <y1> <x2> <y2> - Swipe from (x1,y1) to (x2,y2)");
    println!("  help                     - Show this help message");
    println!("  quit | exit              - Exit the server");
    println!();
}

/// Execute command and broadcast events to clients
async fn execute_command(
    command: Command,
    clients: &Arc<Mutex<Vec<ClientConnection>>>,
) -> Result<bool, Box<dyn std::error::Error>> {
    match command {
        Command::ButtonPress(index) => {
            broadcast_button_event(clients, index, true).await?;
            log(LogLevel::Info, &format!("[Command] Button {} pressed", index));
            Ok(false)
        }
        Command::ButtonRelease(index) => {
            broadcast_button_event(clients, index, false).await?;
            log(LogLevel::Info, &format!("[Command] Button {} released", index));
            Ok(false)
        }
        Command::EncoderRotate(index, delta) => {
            broadcast_encoder_rotate_event(clients, index, delta).await?;
            log(LogLevel::Info, &format!("[Command] Encoder {} rotated by {}", index, delta));
            Ok(false)
        }
        Command::EncoderPress(index) => {
            broadcast_encoder_press_event(clients, index, true).await?;
            log(LogLevel::Info, &format!("[Command] Encoder {} pressed", index));
            Ok(false)
        }
        Command::EncoderRelease(index) => {
            broadcast_encoder_press_event(clients, index, false).await?;
            log(LogLevel::Info, &format!("[Command] Encoder {} released", index));
            Ok(false)
        }
        Command::Touch(touch_type, x, y, end_x, end_y) => {
            let touch_name = match touch_type {
                0x01 => "tap",
                0x02 => "press",
                0x03 => "swipe",
                _ => "unknown",
            };
            broadcast_touch_event(clients, touch_type, x, y, end_x, end_y).await?;
            if touch_type == 0x03 {
                log(LogLevel::Info, &format!("[Command] Touch {} from ({},{}) to ({},{})", 
                    touch_name, x, y, end_x.unwrap_or(0), end_y.unwrap_or(0)));
            } else {
                log(LogLevel::Info, &format!("[Command] Touch {} at ({},{})", touch_name, x, y));
            }
            Ok(false)
        }
        Command::Help => {
            print_help();
            Ok(false)
        }
        Command::Quit => {
            log(LogLevel::Info, "[Command] Shutting down server...");
            Ok(true)
        }
    }
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

    println!("Stream Deck NetworkDock Emulator (Cora Protocol)");
    println!("Listening on {}...", addr);
    println!("Connect clients to this address");
    println!("Type 'help' for available commands");
    println!("Press Ctrl+C or type 'quit' to stop\n");

    let state = Arc::new(RwLock::new(DeviceState::default()));
    let clients: Arc<Mutex<Vec<ClientConnection>>> = Arc::new(Mutex::new(Vec::new()));

    // Handle Ctrl+C signal
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        log(LogLevel::Info, "\n[Server] Received Ctrl+C, shutting down...");
        std::process::exit(0);
    });

    // Handle standard input commands
    let clients_stdin = clients.clone();
    tokio::spawn(async move {
        let stdin = tokio::io::stdin();
        let mut reader = BufReader::new(stdin);
        let mut line = String::new();

        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => {
                    // EOF
                    break;
                }
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    match parse_command(trimmed) {
                        Ok(cmd) => {
                            if let Ok(should_quit) = execute_command(cmd, &clients_stdin).await {
                                if should_quit {
                                    std::process::exit(0);
                                }
                            }
                        }
                        Err(e) => {
                            log_error(&format!("[Error] {}", e));
                        }
                    }
                }
                Err(e) => {
                    log_error(&format!("[Error] Failed to read from stdin: {}", e));
                    break;
                }
            }
        }
    });

    // Main server loop - accept connections
    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                log(LogLevel::Info, &format!("[Server] New connection from {}", addr));
                let state_clone = state.clone();
                let clients_clone = clients.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_client(stream, state_clone, clients_clone).await {
                        log_error(&format!("[Server] Error handling client: {}", e));
                    }
                });
            }
            Err(e) => {
                log_error(&format!("[Server] Accept error: {}", e));
            }
        }
    }
    // Note: The loop above never exits, so this return is unreachable
    // but required by the function signature
    #[allow(unreachable_code)]
    Ok(())
}
