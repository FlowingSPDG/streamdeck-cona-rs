//! Stream Deck Studio Server Emulator (Cora Protocol)
//!
//! This is a CUI tool that implements a TCP server that emulates a Stream Deck Studio device.
//! It uses the Cora protocol for communication.
//!
//! Usage: streamdeck-server [PORT]

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
                log(LogLevel::Debug, &format!("... ({} messages skipped)", skipped));
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
    button_states: [bool; 32], // Button states (true = pressed, false = released)
}

impl Default for DeviceState {
    fn default() -> Self {
        // Generate unique serial number to avoid cache issues
        // Format: EMULATOR_<timestamp>_<random>
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let random_part = (timestamp % 10000) as u32;
        let serial_number = format!("EMULATOR_STUDIO_{:08X}", random_part);
        
        Self {
            serial_number,
            firmware_version: "1.05.008".to_string(),
            brightness: 100,
            button_images: Default::default(),
            encoder_colors: [(0, 0, 0), (0, 0, 0)],
            device2_connected: false, // Device 2 starts disconnected
            button_states: [false; 32], // All buttons start released
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
    writer.flush().await?;
    
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
    writer.flush().await?;
    
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
    writer.flush().await?;
    
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
/// Per PROTOCOL.md: payload is [0x01, 0x0a, ...] with connection_no at index 5
/// Cora protocol uses variable-length payloads (not fixed 1024 bytes)
async fn send_keep_alive(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    next_message_id: &Arc<Mutex<u32>>,
    connection_no: &Arc<Mutex<u8>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut msg_id = next_message_id.lock().await;
    let conn_no = *connection_no.lock().await;
    
    // Keep-alive payload: [0x01, 0x0a, ...] with connection_no at index 5
    // 32 bytes is sufficient for keep-alive (Cora protocol uses variable-length payloads)
    let keep_alive_payload = {
        let mut payload = vec![0u8; 32];
        payload[0] = 0x01;
        payload[1] = 0x0a;
        payload[5] = conn_no;  // Connection number at index 5 (per PROTOCOL.md)
        payload
    };

    log(LogLevel::Debug, &format!(
        "[Server] Sending keep-alive: msg_id={}, conn_no={} (payload[5]={}), payload=[01, 0a, ...]",
        *msg_id, conn_no, keep_alive_payload[5]
    ));

    let keep_alive_message = CoraMessage::new(
        CoraMessageFlags::None,
        CoraHidOp::SendReport,
        *msg_id,
        keep_alive_payload,
    );
    *msg_id += 1;

    let keep_alive_data = keep_alive_message.encode();
    writer.write_all(&keep_alive_data).await?;
    writer.flush().await?;
    
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

            // Read payload length from header (Little Endian - confirmed)
            // Cora protocol uses variable-length payloads
            let payload_length = u32::from_le_bytes([
                receive_buffer[12], receive_buffer[13], receive_buffer[14], receive_buffer[15]
            ]) as usize;
            
            // Log header details for debugging
            log(LogLevel::Debug, &format!(
                "[Server] Header: payload_length={} (LE, raw bytes: {:02x} {:02x} {:02x} {:02x}), buffer_len={}, message_size={}",
                payload_length, receive_buffer[12], receive_buffer[13], receive_buffer[14], receive_buffer[15],
                receive_buffer.len(), CORA_HEADER_SIZE + payload_length
            ));

            // Check if we have full message (Cora protocol uses variable-length payloads)
            if receive_buffer.len() < CORA_HEADER_SIZE + payload_length {
                break;
            }

            let message_bytes = &receive_buffer[..CORA_HEADER_SIZE + payload_length];

            // Decode Cora message (variable-length payload)
            match CoraMessage::decode(message_bytes) {
                Ok(message) => {
                    // Log received message details
                    let payload_preview = if message.payload.len() <= 16 {
                        format!("{:02x?}", message.payload)
                    } else {
                        format!("{:02x?}...", &message.payload[..16])
                    };
                    // Get raw flags value for debugging (check if VERBATIM flag is set)
                    let raw_flags_bytes = if receive_buffer.len() >= 6 {
                        format!("{:02x} {:02x}", receive_buffer[4], receive_buffer[5])
                    } else {
                        "??".to_string()
                    };
                    let raw_flags_value = u16::from_le_bytes([
                        if receive_buffer.len() >= 6 { receive_buffer[4] } else { 0 },
                        if receive_buffer.len() >= 6 { receive_buffer[5] } else { 0 }
                    ]);
                    
                    log(LogLevel::Info, &format!(
                        "[Server] Decoded message: hid_op={:?}, flags={:?} (raw: 0x{:04x}, bytes: {}), msg_id={}, payload_len={}, payload={}", 
                        message.hid_op, message.flags, raw_flags_value, raw_flags_bytes, message.message_id, message.payload.len(), payload_preview
                    ));

                    // Remove processed message from buffer
                    receive_buffer.drain(..CORA_HEADER_SIZE + payload_length);

                    // Handle message
                    {
                        let mut writer_guard = writer_shared.lock().await;
                        match handle_message(&message, &mut *writer_guard, &state, &next_message_id, &connection_no, &mut throttled_logger).await {
                            Ok(()) => {
                                // Message handled successfully
                                // Update last data time after processing message (including keep-alive ACK)
                                _last_data_time = std::time::Instant::now();
                            }
                            Err(e) => {
                                // Error handling message - log but continue processing
                                // Don't disconnect on single message error (could be transient)
                                let error_str = e.to_string();
                                log_error(&format!("[Server] Error handling message (hid_op={:?}, flags={:?}, msg_id={}): {} (continuing)", 
                                    message.hid_op, message.flags, message.message_id, error_str));
                                
                                // Check if it's a critical error that requires disconnection
                                if error_str.contains("Broken pipe") || error_str.contains("Connection reset") || error_str.contains("Connection aborted") {
                                    log_error(&format!("[Server] Critical connection error detected, will disconnect"));
                                    break;
                                }
                                // For other errors, continue processing but don't update last data time
                                // This allows keep-alive to detect stale connections
                            }
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
/// 
/// Primary port format: [0x03, report_id] (flags: NONE = 0x0000)
/// Secondary port format: [report_id] (flags: VERBATIM = 0x8000)
fn parse_get_report_request(payload: &[u8], flags: CoraMessageFlags) -> Option<u8> {
    // Check flags to determine port format
    // VERBATIM flag (0x8000) indicates secondary port format
    if flags == CoraMessageFlags::Verbatim {
        // Secondary port format: [report_id] (flags: VERBATIM = 0x8000)
        if payload.len() >= 1 {
            Some(payload[0])
        } else {
            None
        }
    } else {
        // Primary port format: [0x03, report_id] (flags: NONE = 0x0000)
        // Also handles other flags (ReqAck, AckNak, Result) as primary port format
        if payload.len() >= 2 && payload[0] == 0x03 {
            Some(payload[1])
        } else {
            None
        }
    }
}

/// Build GET_REPORT response payload in standard format
/// Build GET_REPORT response payload following HID Feature Report structure
/// Based on Elgato HID API documentation: https://docs.elgato.com/streamdeck/hid/module-15_32
/// 
/// USB HID Feature Report structure (from official docs):
///   Offset 0: Report ID
///   Offset 1: Data Length (or Command)
///   Offset 2+: Payload data
/// 
/// TCP/Cora protocol wraps this as:
///   Format: [0x03, report_id, length_high, length_low, ...data...]
///   - 0x03: Fixed prefix for Feature Report in Cora protocol
///   - report_id: Feature Report ID (e.g., 0x83=firmware, 0x84=serial, 0x85=MAC)
///   - length_high, length_low: Big Endian length of data (unlike other fields which are Little Endian)
///   - data: Actual response data following HID Feature Report format
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
        // Stream Deck Studio product ID: 0x00aa
        // According to @elgato-stream-deck/core: Studio productIds: [0x00aa]
        let product_id: u16 = 0x00aa;
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
    writer.flush().await?;
    
    Ok(())
}

/// Get data for a specific report ID from device state
/// Build primary port device info payload (Report ID 0x80)
/// This data will be wrapped by build_get_report_response_payload as:
///   [0x03, 0x80, length_high, length_low, ...data...]
/// TypeScript client (cora.ts:161-162) reads vendorId/productId from the wrapped payload at offset 12-15:
///   const vendorId = dataView.getUint16(12, true)  // Little Endian
///   const productId = dataView.getUint16(14, true) // Little Endian
/// The wrapped payload structure:
///   Offset 0-1: [0x03, 0x80] (2 bytes)
///   Offset 2-3: [len_high, len_low] (2 bytes, Big Endian)
///   Offset 4+: data (16 bytes)
/// So offset 12-13 in wrapped payload = data[8-9], offset 14-15 = data[10-11]
/// We need to put VID at data[8-9] and PID at data[10-11] in Little Endian format
fn build_primary_port_info_payload(vendor_id: u16, product_id: u16) -> Vec<u8> {
    let mut payload = vec![0u8; 16];
    // Vendor ID at data offset 8-9 (Little Endian: lower byte first)
    // After wrapping: will be at payload offset 12-13
    payload[8] = (vendor_id & 0xff) as u8;       // Lower byte
    payload[9] = ((vendor_id >> 8) & 0xff) as u8; // Upper byte
    // Product ID at data offset 10-11 (Little Endian: lower byte first)
    // After wrapping: will be at payload offset 14-15
    payload[10] = (product_id & 0xff) as u8;       // Lower byte
    payload[11] = ((product_id >> 8) & 0xff) as u8; // Upper byte
    payload
}

async fn get_report_data(state: &Arc<RwLock<DeviceState>>, report_id: u8) -> Option<Vec<u8>> {
    match report_id {
        0x80 => {
            // Primary port device info (Studio port)
            // Vendor ID: 0x0fd9 (Elgato Systems GmbH)
            // Product ID: 0x00aa (Stream Deck Studio)
            // According to @elgato-stream-deck/core: Studio productIds: [0x00aa]
            // For Studio, we must respond to 0x80 to be recognized as primary port
            Some(build_primary_port_info_payload(0x0fd9, 0x00aa))
        }
        0x08 => {
            // Secondary port device info (Device 2 port) - Report ID 0x08
            // Based on HID Feature Report structure from official docs:
            //   USB HID: [Report ID (0x08), Data Length, ...device info...]
            //   TCP/Cora wraps as: [0x03, 0x08, len_high, len_low, ...USB HID data...]
            // 
            // TypeScript client behavior:
            // - Uses Promise.race to send both 0x80 and 0x08 concurrently
            // - First response determines if it's primary (0x80) or secondary (0x08) port
            // - If 0x08 responds, client sets isPrimary=false, then queries 0x1c for Device 2 info
            // - Even if Device 2 is not connected, we should respond to acknowledge the request
            // 
            // NOTE: The exact structure of 0x08 response is unclear from client code.
            // The client only checks if 0x08 responds (to determine secondary port),
            // but doesn't parse vendorId/productId from 0x08 - it uses 0x1c for that.
            // For now, we return the same structure as 0x80 to acknowledge the request.
            // TODO: Verify actual 0x08 response structure from real device behavior
            Some(build_primary_port_info_payload(0x0fd9, 0x00aa))
        }
        0x83 => {
            // Firmware version (AP2) - Report ID 0x83
            // Based on HID Feature Report structure from official docs:
            //   USB HID: [Report ID (0x05), Data Length (0x0C), Checksum (UINT32), Version String (UINT8[8])]
            //   TCP/Cora wraps as: [0x03, 0x83, len_high, len_low, ...USB HID data...]
            // 
            // TypeScript client expectations:
            // - getFirmwareVersion() reads data.subarray(8, 16) = data[4] after wrapping (0x03+report_id+length)
            // - getAllFirmwareVersions() uses ap2Data.slice(2), then parseAllFirmwareVersionsHelper reads subarray(6, 6+8)
            //   This means it expects version at offset 6 after slice(2), which is data[2] in original data
            // 
            // To support both, we place firmware version at both data[2] and data[4]
            // This data will be wrapped by build_get_report_response_payload as:
            //   [0x03, 0x83, 0x00, 0x10, ...fw_data...] where fw_data[2] and fw_data[4] contain version
            let device_state = state.read().await;
            let fw_version = device_state.firmware_version.clone();
            drop(device_state);
            
            let mut fw_data = vec![0u8; 16];
            let fw_bytes = fw_version.as_bytes();
            let copy_len = fw_bytes.len().min(8);
            // Put firmware version at offset 2 (for getAllFirmwareVersions) and offset 4 (for getFirmwareVersion)
            fw_data[2..2 + copy_len].copy_from_slice(&fw_bytes[..copy_len]);
            fw_data[4..4 + copy_len].copy_from_slice(&fw_bytes[..copy_len]);
            Some(fw_data)
        }
        0x86 | 0x8a => {
            // Encoder firmware version (AP2 = 0x86, LD = 0x8a)
            // TypeScript: encoderAp2Data.slice(2) or encoderLdData.slice(2), then parseAllFirmwareVersionsHelper reads subarray(2, 2+8)
            // Response format: [0x03, 0x86/0x8a, len_high, len_low, ...data...]
            // After slice(2): [len_high, len_low, ...data...]
            // parseAllFirmwareVersionsHelper reads offset 2 from sliced data, which is data[0] in original data
            // Also checks data[0] === 0x18 || data[1] === 0x18
            // So firmware version should be at data[0] (which becomes offset 2 after slice(2))
            // And we need 0x18 at data[0] or data[1] for the check to pass
            let device_state = state.read().await;
            let fw_version = device_state.firmware_version.clone();
            drop(device_state);
            
            let mut fw_data = vec![0u8; 32];
            let fw_bytes = fw_version.as_bytes();
            let copy_len = fw_bytes.len().min(8);
            // Put firmware version at data[0] (offset 2 after slice(2))
            // Put 0x18 at data[1] to pass the check (since firmware version starts at data[0])
            fw_data[0..copy_len].copy_from_slice(&fw_bytes[..copy_len]);
            fw_data[1] = 0x18; // Magic byte for encoder firmware (at offset 1 to pass check)
            Some(fw_data)
        }
        0x84 => {
            // Serial number - Report ID 0x84
            // Based on HID Feature Report structure from official docs:
            //   USB HID: [Report ID (0x06), Data Length (0x0C or 0x0E), Serial String ASCII]
            //   TCP/Cora wraps as: [0x03, 0x84, len_high, len_low, ...serial...]
            // 
            // TypeScript: getSerialNumber() reads data[3] for length (len_low if len_high=0), 
            // then data.subarray(4, 4+length) for the serial string
            // Since we return serial bytes directly, build_get_report_response_payload will add [0x03, 0x84, len_high, len_low]
            // For typical serial numbers (< 256 bytes), len_high=0, so data[3] (len_low) contains the length
            let device_state = state.read().await;
            let serial = device_state.serial_number.clone();
            drop(device_state);
            
            Some(serial.as_bytes().to_vec())
        }
        0x85 => {
            // MAC address
            // TypeScript: getMacAddress() reads data.subarray(4, 10) (6 bytes)
            // Response format: [0x03, 0x85, len_high, len_low, ...mac...]
            // TypeScript reads offset 4-10, which is len_high + len_low + mac[0..5]
            // So MAC address should start at data[0] (which becomes offset 4 after wrapping)
            // MAC address is 6 bytes: [mac0, mac1, mac2, mac3, mac4, mac5]
            // Placeholder MAC address: 00:1A:2B:3C:4D:5E
            Some(vec![0x00, 0x1A, 0x2B, 0x3C, 0x4D, 0x5E])
        }
        0x1c => {
            // Device 2 (Child Device) information - Report ID 0x1c
            // Based on device2Info.ts: parseDevice2Info structure
            //   - Offset 4: Device status (0x02 = connected/OK, 0x00 = disconnected)
            //   - Offset 26-27: vendorId (Little Endian, u16)
            //   - Offset 28-29: productId (Little Endian, u16)
            //   - Offset 94-125: Serial number (ASCII, up to 32 bytes)
            //   - Offset 126-127: TCP port (Little Endian, u16)
            // 
            // TypeScript client: parseDevice2Info checks offset 4 - if != 0x02, returns null
            // Even if Device 2 is not connected, we should return 128-byte payload with offset 4 = 0x00
            // This allows client to properly detect disconnected state (returns null but acknowledges request)
            // 
            // For GET_REPORT responses, we don't include the 0x01 0x0b prefix (that's for plug events)
            let device2_connected = state.read().await.device2_connected;
            let payload = build_device2_info_payload(device2_connected, false);
            Some(payload)
        }
        0x87 => {
            // Button state query (GET_REPORT 0x87)
            // Based on productiondeck USB HID implementation:
            // Studio uses TCP, so Input Report (HID Read) is not available
            // GET_REPORT 0x87 is used to poll button states via Feature Report
            // Format: [0x03, 0x87, len_high, len_low, ...button_states...]
            // Button state format: Similar to Input Report format
            // Module 15/32 format: [0x01, 0x00, length_low, length_high, ...button_states...]
            // Studio has 32 buttons, so we need at least 36 bytes (4 header + 32 button states)
            let device_state = state.read().await;
            let mut button_data = vec![0u8; 36];
            
            // Module 15/32 Input Report format (same as USB HID Input Report)
            // Report ID 0x01, Command 0x00, Length = number of buttons, then states
            button_data[0] = 0x01; // Report ID
            button_data[1] = 0x00; // Command: key state change
            button_data[2] = 32;   // length LSB (32 buttons)
            button_data[3] = 0x00; // length MSB
            
            // Copy button states (1 = pressed, 0 = released)
            for (i, &pressed) in device_state.button_states.iter().enumerate() {
                if i < 32 {
                    button_data[4 + i] = if pressed { 1 } else { 0 };
                }
            }
            
            Some(button_data)
        }
        0x8f => {
            // Device-specific query (0x8f) - should return proper response
            // Based on protocol patterns, return minimal status response
            // Format: [0x03, 0x8f, len_high, len_low, ...data...]
            // Return minimal 4-byte response: [0x00, 0x00, 0x00, 0x00] to acknowledge
            // This will be wrapped as [0x03, 0x8f, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00]
            Some(vec![0x00, 0x00, 0x00, 0x00])
        }
        0x1a => {
            // Keep-alive related (already handled separately, but may come as GET_REPORT)
            // Return empty data to avoid confusion
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
        response_payload.clone(),
    );
    
    let response_data = response.encode();
    
    let payload_preview = if response_payload.len() <= 16 {
        format!("{:02x?}", response_payload)
    } else {
        format!("{:02x?}...", &response_payload[..16])
    };
    log(LogLevel::Info, &format!(
        "[Server] Sending GET_REPORT response: report_id=0x{:02x}, msg_id={}, payload_len={}, payload={}",
        report_id, message_id, response_payload.len(), payload_preview
    ));
    
    let encoded_preview = if response_data.len() <= 20 {
        format!("{:02x?}", response_data)
    } else {
        format!("{:02x?}...", &response_data[..20])
    };
    log(LogLevel::Info, &format!(
        "[Server] Encoded message: len={}, data={}",
        response_data.len(), encoded_preview
    ));
    
    writer.write_all(&response_data).await?;
    writer.flush().await?;
    
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
    // Handle keep-alive ACK (0x03 0x1a)
    // Per PROTOCOL.md: ACK response has flags=ACK_NAK (0x0200) and payload=[0x03, 0x1a, connection_no, ...] (32 bytes)
    // Cora protocol uses variable-length payloads (not fixed 1024 bytes)
    // Connection_no can be 0, which is valid data (not padding)
    // This is the most frequently sent message (every 4 seconds) - critical for connection stability
    if message.payload.len() >= 3 && message.payload[0] == 0x03 && message.payload[1] == 0x1a {
        // Extract connection_no (at index 2, can be 0 which is valid)
        let conn_no_value = message.payload[2];
        
        // Get current connection number before update
        let old_conn_no = {
            let conn_no_guard = connection_no.lock().await;
            *conn_no_guard
        };
        
        // Update connection number from keep-alive ACK (even if flags/hid_op are unexpected)
        // This is critical for proper keep-alive handling - connection_no cycles through values
        {
            let mut conn_no = connection_no.lock().await;
            *conn_no = conn_no_value;
        }
        
        // Log connection number change if it's different (helps debug connection_no cycling)
        if conn_no_value != old_conn_no {
            log(LogLevel::Info, &format!(
                "[Server] Keep-alive ACK: connection_no CHANGED from {} to {} (flags={:?}, hid_op={:?})",
                old_conn_no, conn_no_value, message.flags, message.hid_op
            ));
        } else {
            // Connection number unchanged (could be 0 or same value)
            // Only log at debug level to avoid spam since this happens every 4 seconds
            log(LogLevel::Debug, &format!(
                "[Server] Keep-alive ACK: connection_no={} (unchanged), flags={:?}, hid_op={:?}",
                conn_no_value, message.flags, message.hid_op
            ));
        }
        
        // Always return Ok() for keep-alive ACK (don't process further)
        return Ok(());
    }

    // Handle GET_REPORT requests regardless of HID operation
    // Primary port format: [0x03, report_id] (flags: NONE = 0x0000)
    // Secondary port format: [report_id] (flags: VERBATIM = 0x8000)
    // NOTE: Some clients send GET_REPORT requests with hid_op=Write, so we detect by payload content
    
    // Try to parse GET_REPORT request from payload
    // This handles both primary port ([0x03, report_id]) and secondary port ([report_id]) formats
    if let Some(report_id) = parse_get_report_request(&message.payload, message.flags) {
        // Log the detection with detailed flag information
        let flags_value = message.flags as u16;
        let is_verbatim = message.flags == CoraMessageFlags::Verbatim;
        let port_type = if is_verbatim { "Secondary" } else { "Primary" };
        
        // Special logging for critical report IDs (0x80 and 0x08) - these are sent during initialization
        if report_id == 0x80 || report_id == 0x08 {
            log(LogLevel::Warn, &format!(
                "[Server] *** INITIALIZATION GET_REPORT DETECTED ***: report_id=0x{:02x}, flags={:?} (0x{:04x}), port={}, hid_op={:?}, payload_preview={:02x?}",
                report_id, message.flags, flags_value, port_type, message.hid_op,
                if message.payload.len() >= 2 { &message.payload[..2] } else { &message.payload }
            ));
        }
        
        if message.hid_op != CoraHidOp::GetReport {
            log(LogLevel::Info, &format!(
                "[Server] Detected GET_REPORT request by payload: report_id=0x{:02x}, flags={:?} (0x{:04x}), port={}, hid_op={:?} (not GetReport) - treating as GET_REPORT based on payload content",
                report_id, message.flags, flags_value, port_type, message.hid_op
            ));
        } else {
            log(LogLevel::Info, &format!(
                "[Server] Received GET_REPORT request: report_id=0x{:02x}, flags={:?} (0x{:04x}), port={}, hid_op={:?}",
                report_id, message.flags, flags_value, port_type, message.hid_op
            ));
        }
        
        // Report ID 0x80 (primary port) and 0x08 (secondary port) are both used for device detection
        // Client uses Promise.race to send both requests concurrently
        // Both should respond to acknowledge the request, even if Device 2 is not connected
        // The first response received determines whether it's detected as primary or secondary port
        
        match report_id {
            0x80 => {
                // PRIMARY PORT: Respond with Studio device info
                log(LogLevel::Info, "[Server] Responding to GET_REPORT 0x80 (Primary Port) with Studio device info");
                let data = build_primary_port_info_payload(0x0fd9, 0x00aa);
                send_get_report_response(writer, message.message_id, report_id, data).await?;
                return Ok(());
            }
            0x08 => {
                // SECONDARY PORT: Respond to acknowledge the request
                // Based on TypeScript client behavior, we should respond even if Device 2 is not connected
                // The client uses Promise.race and checks if 0x08 responds to determine secondary port
                // After receiving 0x08 response, client queries 0x1c for actual Device 2 info
                // 
                // NOTE: Exact 0x08 response structure is unclear - client doesn't parse vendorId/productId from it
                // For now, we return same structure as 0x80 to acknowledge (TODO: verify real device behavior)
                log(LogLevel::Info, "[Server] Responding to GET_REPORT 0x08 (Secondary Port) to acknowledge request");
                // TODO: Verify correct 0x08 response structure - client uses 0x1c for actual Device 2 info
                let data = build_primary_port_info_payload(0x0fd9, 0x00aa);
                send_get_report_response(writer, message.message_id, report_id, data).await?;
                return Ok(());
            }
            0x1a => {
                // Report ID 0x1a: Keep-alive related, should NOT come as GET_REPORT
                // If it does, it's likely a keep-alive ACK that was misidentified
                // Don't respond as GET_REPORT - it's already handled as keep-alive ACK above
                log(LogLevel::Warn, &format!(
                    "[Server] GET_REPORT 0x1a received (unexpected - should be keep-alive ACK). Ignoring as GET_REPORT."
                ));
                return Ok(());
            }
            _ => {
                // Other report IDs - handle normally
                if let Some(data) = get_report_data(state, report_id).await {
                    // Check if data is empty (which indicates unsupported or optional feature)
                    if data.is_empty() {
                        log(LogLevel::Info, &format!("[Server] GET_REPORT 0x{:02x} - returning empty response (optional/unsupported)", report_id));
                        // Still send response with empty data to acknowledge the request
                        send_get_report_response(writer, message.message_id, report_id, data).await?;
                    } else {
                        log(LogLevel::Info, &format!("[Server] Responding to GET_REPORT 0x{:02x}", report_id));
                        send_get_report_response(writer, message.message_id, report_id, data).await?;
                    }
                    return Ok(());
                } else {
                    log(LogLevel::Warn, &format!("[Server] GET_REPORT 0x{:02x} - no data available, request ignored", report_id));
                    // Don't send response for unsupported report IDs to avoid confusing the client
                    // Just return Ok() without response
                    return Ok(());
                }
            }
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
                        log(LogLevel::Info, &format!("[Server] SEND_REPORT: Set brightness to {}%", brightness));
                    }

                    // Reset (0x03 0x02)
                    (0x03, 0x02) => {
                        *state.write().await = DeviceState::default();
                        log(LogLevel::Info, "[Server] SEND_REPORT: Reset device");
                    }

                    _ => {
                        // Unknown SEND_REPORT command
                        log(LogLevel::Info, &format!("[Server] SEND_REPORT: Unknown command 0x{:02x} 0x{:02x}", 
                            message.payload[0],
                            if message.payload.len() > 1 { message.payload[1] } else { 0 }));
                    }
                }
            } else {
                log(LogLevel::Info, &format!("[Server] SEND_REPORT: Payload too short (len={})", message.payload.len()));
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
                        // Log only occasionally to avoid spam
                        throttled_logger.log(LogLevel::Debug, "[Server] WRITE: Display area image data");
                    }

                    _ => {
                        // Unknown WRITE command
                        log(LogLevel::Info, &format!("[Server] WRITE: Unknown command 0x{:02x} 0x{:02x}", 
                            message.payload[0],
                            if message.payload.len() > 1 { message.payload[1] } else { 0 }));
                    }
                }
            } else {
                log(LogLevel::Info, &format!("[Server] WRITE: Payload too short (len={})", message.payload.len()));
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
    state: &Arc<RwLock<DeviceState>>,
) -> Result<bool, Box<dyn std::error::Error>> {
    match command {
        Command::ButtonPress(index) => {
            // Update button state in DeviceState
            state.write().await.button_states[index as usize] = true;
            broadcast_button_event(clients, index, true).await?;
            log(LogLevel::Info, &format!("[Command] Button {} pressed", index));
            Ok(false)
        }
        Command::ButtonRelease(index) => {
            // Update button state in DeviceState
            state.write().await.button_states[index as usize] = false;
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

    log(LogLevel::Info, "Stream Deck Studio Emulator (Cora Protocol)");
    log(LogLevel::Info, &format!("Listening on {}...", addr));
    log(LogLevel::Info, "Connect clients to this address");
    log(LogLevel::Info, "Type 'help' for available commands");
    log(LogLevel::Info, "Press Ctrl+C or type 'quit' to stop");
    println!();

    let state = Arc::new(RwLock::new(DeviceState::default()));
    {
        // Log device information on startup
        let device_state = state.read().await;
        log(LogLevel::Info, &format!("[Device] Serial Number: {}", device_state.serial_number));
        log(LogLevel::Info, &format!("[Device] Firmware Version: {}", device_state.firmware_version));
        log(LogLevel::Info, &format!("[Device] Vendor ID: 0x0fd9 (Elgato Systems GmbH)"));
        log(LogLevel::Info, &format!("[Device] Product ID: 0x00aa (Stream Deck Studio)"));
        log(LogLevel::Info, &format!("[Device] Device Type: Studio (Primary Port, Report ID 0x80)"));
    }
    let clients: Arc<Mutex<Vec<ClientConnection>>> = Arc::new(Mutex::new(Vec::new()));

    // Handle Ctrl+C signal
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        log(LogLevel::Info, "\n[Server] Received Ctrl+C, shutting down...");
        std::process::exit(0);
    });

    // Handle standard input commands
    let clients_stdin = clients.clone();
    let state_stdin = state.clone();
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
                            if let Ok(should_quit) = execute_command(cmd, &clients_stdin, &state_stdin).await {
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
