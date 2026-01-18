//! Stream Deck Manager - CUI Application for Managing Multiple Virtual Stream Deck Studio Instances
//!
//! This is a CUI tool that allows you to create, delete, list, and control multiple virtual Stream Deck Studio devices.
//!
//! Usage: streamdeck-manager

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex, RwLock};
use streamdeck_cora_rs::protocol::{CoraMessage, CoraMessageFlags, CoraHidOp, CORA_MAGIC, CORA_HEADER_SIZE};

const BASE_PORT: u16 = 5343;

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

/// Print message with timestamp
fn print_msg(message: &str) {
    println!("{} {}", format_time(), message);
}

/// Print error message with timestamp
fn print_error(message: &str) {
    eprintln!("{} [ERROR] {}", format_time(), message);
}

/// Device state for a virtual Stream Deck
#[derive(Debug, Clone)]
struct DeviceState {
    serial_number: String,
    firmware_version: String,
    brightness: u8,
    button_images: [Option<Vec<u8>>; 32],
    encoder_colors: [(u8, u8, u8); 2],
    device2_connected: bool,
}

impl DeviceState {
    fn new(serial_number: String) -> Self {
        Self {
            serial_number,
            firmware_version: "1.05.008".to_string(),
            brightness: 100,
            button_images: Default::default(),
            encoder_colors: [(0, 0, 0), (0, 0, 0)],
            device2_connected: false,
        }
    }
}

/// Client connection information
struct ClientConnection {
    writer: Arc<Mutex<tokio::net::tcp::OwnedWriteHalf>>,
    next_message_id: Arc<Mutex<u32>>,
}

/// Control command from manager
#[derive(Debug, Clone)]
enum ControlCommand {
    ButtonPress(u8),
    ButtonRelease(u8),
    EncoderRotate(u8, i8),
    EncoderPress(u8),
    EncoderRelease(u8),
    Touch(u8, u16, u16, Option<u16>, Option<u16>),
}

/// Virtual Stream Deck server instance
struct VirtualStreamDeck {
    serial_number: String,
    port: u16,
    state: Arc<RwLock<DeviceState>>,
    clients: Arc<Mutex<Vec<ClientConnection>>>,
    control_tx: mpsc::Sender<ControlCommand>,
    handle: tokio::task::JoinHandle<()>,
}

impl VirtualStreamDeck {
    /// Get serial number
    fn serial(&self) -> &str {
        &self.serial_number
    }

    /// Get port
    fn port(&self) -> u16 {
        self.port
    }

    /// Send control command
    async fn send_control(&self, cmd: ControlCommand) -> Result<(), Box<dyn std::error::Error>> {
        self.control_tx.send(cmd).await?;
        Ok(())
    }

    /// Stop the virtual device
    fn stop(self) {
        self.handle.abort();
    }
}

/// Send button event to client
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
    payload[4] = 0x01;
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
    payload[4] = 0x00;
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
async fn send_touch_event(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    next_message_id: &Arc<Mutex<u32>>,
    touch_type: u8,
    x: u16,
    y: u16,
    end_x: Option<u16>,
    end_y: Option<u16>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut msg_id = next_message_id.lock().await;
    
    let payload = if touch_type == 0x03 && end_x.is_some() && end_y.is_some() {
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
async fn broadcast_event(
    clients: &Arc<Mutex<Vec<ClientConnection>>>,
    cmd: &ControlCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    let clients_guard = clients.lock().await;
    let mut disconnected = Vec::new();
    
    for (i, client) in clients_guard.iter().enumerate() {
        let mut writer = client.writer.lock().await;
        let result = match cmd {
            ControlCommand::ButtonPress(idx) => {
                send_button_event(&mut *writer, &client.next_message_id, *idx, true).await
            }
            ControlCommand::ButtonRelease(idx) => {
                send_button_event(&mut *writer, &client.next_message_id, *idx, false).await
            }
            ControlCommand::EncoderRotate(idx, delta) => {
                send_encoder_rotate_event(&mut *writer, &client.next_message_id, *idx, *delta).await
            }
            ControlCommand::EncoderPress(idx) => {
                send_encoder_press_event(&mut *writer, &client.next_message_id, *idx, true).await
            }
            ControlCommand::EncoderRelease(idx) => {
                send_encoder_press_event(&mut *writer, &client.next_message_id, *idx, false).await
            }
            ControlCommand::Touch(tt, x, y, ex, ey) => {
                send_touch_event(&mut *writer, &client.next_message_id, *tt, *x, *y, *ex, *ey).await
            }
        };
        
        if result.is_err() {
            disconnected.push(i);
        }
    }
    
    // Remove disconnected clients (in reverse order to maintain indices)
    drop(clients_guard);
    let mut clients_guard = clients.lock().await;
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

/// Build primary port device info payload
fn build_primary_port_info_payload(vendor_id: u16, product_id: u16) -> Vec<u8> {
    let mut payload = vec![0u8; 16];
    payload[8] = (vendor_id & 0xff) as u8;
    payload[9] = ((vendor_id >> 8) & 0xff) as u8;
    payload[10] = (product_id & 0xff) as u8;
    payload[11] = ((product_id >> 8) & 0xff) as u8;
    payload
}

/// Build GET_REPORT response payload
fn build_get_report_response_payload(report_id: u8, data: &[u8]) -> Vec<u8> {
    let len = data.len().min(65535) as u16;
    let mut payload = Vec::with_capacity(4 + len as usize);
    
    payload.push(0x03);
    payload.push(report_id);
    payload.push(((len >> 8) & 0xff) as u8);
    payload.push((len & 0xff) as u8);
    payload.extend_from_slice(&data[..len as usize]);
    
    payload
}

/// Parse GET_REPORT request
fn parse_get_report_request(payload: &[u8], flags: CoraMessageFlags) -> Option<u8> {
    if flags == CoraMessageFlags::Verbatim {
        if payload.len() >= 1 {
            Some(payload[0])
        } else {
            None
        }
    } else {
        if payload.len() >= 2 && payload[0] == 0x03 {
            Some(payload[1])
        } else {
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

/// Handle message from client
async fn handle_message(
    message: &CoraMessage,
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    state: &Arc<RwLock<DeviceState>>,
    connection_no: &Arc<Mutex<u8>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Handle keep-alive ACK
    if message.flags == CoraMessageFlags::AckNak && message.payload.len() >= 3 && message.payload[0] == 0x03 && message.payload[1] == 0x1a {
        let mut conn_no = connection_no.lock().await;
        *conn_no = message.payload[2];
        return Ok(());
    }

    // Handle GET_REPORT requests
    if let Some(report_id) = parse_get_report_request(&message.payload, message.flags) {
        match report_id {
            0x80 => {
                let data = build_primary_port_info_payload(0x0fd9, 0x00aa);
                send_get_report_response(writer, message.message_id, report_id, data).await?;
                return Ok(());
            }
            0x08 => {
                // Don't respond to secondary port for Studio
                return Ok(());
            }
            0x83 => {
                let data = state.read().await.firmware_version.as_bytes().to_vec();
                send_get_report_response(writer, message.message_id, report_id, data).await?;
                return Ok(());
            }
            0x84 => {
                let data = state.read().await.serial_number.as_bytes().to_vec();
                send_get_report_response(writer, message.message_id, report_id, data).await?;
                return Ok(());
            }
            _ => {
                // Unknown report ID - return empty
                send_get_report_response(writer, message.message_id, report_id, Vec::new()).await?;
                return Ok(());
            }
        }
    }

    // Handle other commands
    match message.hid_op {
        CoraHidOp::SendReport => {
            if message.payload.len() >= 2 {
                match (message.payload[0], message.payload[1]) {
                    (0x03, 0x08) if message.payload.len() >= 3 => {
                        let brightness = message.payload[2];
                        state.write().await.brightness = brightness;
                    }
                    (0x03, 0x02) => {
                        // Reset - keep serial number
                        let serial = state.read().await.serial_number.clone();
                        *state.write().await = DeviceState::new(serial);
                    }
                    _ => {}
                }
            }
        }
        CoraHidOp::Write => {
            // Handle image writes, etc.
        }
        _ => {}
    }

    Ok(())
}

/// Handle client connection
async fn handle_client(
    stream: TcpStream,
    state: Arc<RwLock<DeviceState>>,
    clients: Arc<Mutex<Vec<ClientConnection>>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut reader, writer) = stream.into_split();
    let mut receive_buffer = Vec::new();
    let next_message_id = Arc::new(Mutex::new(0u32));
    let connection_no = Arc::new(Mutex::new(0u8));
    let writer_shared = Arc::new(Mutex::new(writer));

    // Send initial keep-alive
    {
        let mut writer_guard = writer_shared.lock().await;
        send_keep_alive(&mut *writer_guard, &next_message_id, &connection_no).await?;
    }

    // Register client
    let client_conn = ClientConnection {
        writer: writer_shared.clone(),
        next_message_id: next_message_id.clone(),
    };
    {
        let mut clients_guard = clients.lock().await;
        clients_guard.push(client_conn);
    }

    // Keep-alive interval
    let mut keep_alive_interval = tokio::time::interval(std::time::Duration::from_secs(4));
    keep_alive_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    keep_alive_interval.tick().await;

    let mut read_chunk = vec![0u8; 4096];
    
    loop {
        tokio::select! {
            _ = keep_alive_interval.tick() => {
                let mut writer_guard = writer_shared.lock().await;
                if send_keep_alive(&mut *writer_guard, &next_message_id, &connection_no).await.is_err() {
                    break;
                }
            }
            result = tokio::time::timeout(std::time::Duration::from_secs(10), reader.read(&mut read_chunk)) => {
                match result {
                    Ok(Ok(0)) => break,
                    Ok(Ok(n)) => {
                        receive_buffer.extend_from_slice(&read_chunk[..n]);
                    }
                    Ok(Err(_)) => break,
                    Err(_) => continue,
                }
            }
        }

        // Process messages
        while receive_buffer.len() >= CORA_HEADER_SIZE {
            let magic_pos = receive_buffer.windows(4).position(|w| w == &CORA_MAGIC);
            
            if let Some(pos) = magic_pos {
                if pos > 0 {
                    receive_buffer.drain(..pos);
                }
            } else {
                if receive_buffer.len() > 3 {
                    let keep = receive_buffer[receive_buffer.len() - 3..].to_vec();
                    receive_buffer.clear();
                    receive_buffer.extend_from_slice(&keep);
                }
                break;
            }

            if receive_buffer.len() < CORA_HEADER_SIZE {
                break;
            }

            let payload_length = u32::from_le_bytes([
                receive_buffer[12], receive_buffer[13], receive_buffer[14], receive_buffer[15]
            ]) as usize;

            if receive_buffer.len() < CORA_HEADER_SIZE + payload_length {
                break;
            }

            match CoraMessage::decode(&receive_buffer[..CORA_HEADER_SIZE + payload_length]) {
                Ok(message) => {
                    receive_buffer.drain(..CORA_HEADER_SIZE + payload_length);
                    let mut writer_guard = writer_shared.lock().await;
                    if let Err(_) = handle_message(&message, &mut *writer_guard, &state, &connection_no).await {
                        // Continue on error
                    }
                }
                Err(_) => {
                    if let Some(pos) = receive_buffer[1..].windows(4).position(|w| w == &CORA_MAGIC) {
                        receive_buffer.drain(..pos + 1);
                    } else {
                        receive_buffer.clear();
                    }
                }
            }
        }
    }

    // Unregister client
    {
        let mut clients_guard = clients.lock().await;
        clients_guard.retain(|c| !Arc::ptr_eq(&c.writer, &writer_shared));
    }
    
    Ok(())
}

/// Run virtual Stream Deck server
async fn run_virtual_streamdeck(
    serial_number: String,
    port: u16,
    mut control_rx: mpsc::Receiver<ControlCommand>,
) {
    let state = Arc::new(RwLock::new(DeviceState::new(serial_number.clone())));
    let clients: Arc<Mutex<Vec<ClientConnection>>> = Arc::new(Mutex::new(Vec::new()));
    let clients_for_handle = clients.clone();

    let addr = format!("0.0.0.0:{}", port);
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            print_error(&format!("Failed to bind {}: {}", addr, e));
            return;
        }
    };

    print_msg(&format!("[{}] Virtual Stream Deck started on {} (Serial: {})", 
        serial_number, addr, serial_number));

    // Spawn task to handle control commands
    let clients_for_control = clients.clone();
    let serial_for_control = serial_number.clone();
    tokio::spawn(async move {
        while let Some(cmd) = control_rx.recv().await {
            if let Err(e) = broadcast_event(&clients_for_control, &cmd).await {
                print_error(&format!("[{}] Error broadcasting event: {}", serial_for_control, e));
            }
        }
    });

    // Main server loop
    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                print_msg(&format!("[{}] Client connected from {}", serial_number, addr));
                let state_clone = state.clone();
                let clients_clone = clients_for_handle.clone();
                let serial_for_client = serial_number.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_client(stream, state_clone, clients_clone).await {
                        print_error(&format!("[{}] Error handling client: {}", serial_for_client, e));
                    }
                });
            }
            Err(e) => {
                print_error(&format!("[{}] Accept error: {}", serial_number, e));
            }
        }
    }
}

/// Stream Deck Manager - manages multiple virtual devices
struct StreamDeckManager {
    devices: Arc<Mutex<HashMap<String, VirtualStreamDeck>>>,
    next_port: Arc<Mutex<u16>>,
}

impl StreamDeckManager {
    fn new() -> Self {
        Self {
            devices: Arc::new(Mutex::new(HashMap::new())),
            next_port: Arc::new(Mutex::new(BASE_PORT)),
        }
    }

    /// Create a new virtual Stream Deck
    async fn create(&self, serial_number: String) -> Result<(), Box<dyn std::error::Error>> {
        let mut devices = self.devices.lock().await;
        
        if devices.contains_key(&serial_number) {
            return Err(format!("Device with serial number '{}' already exists", serial_number).into());
        }

        let port = {
            let mut next = self.next_port.lock().await;
            let p = *next;
            *next += 1;
            p
        };

        let (control_tx, control_rx) = mpsc::channel(100);
        let serial_clone = serial_number.clone();
        let port_clone = port;

        let handle = tokio::spawn(async move {
            run_virtual_streamdeck(serial_clone, port_clone, control_rx).await;
        });

        let device = VirtualStreamDeck {
            serial_number: serial_number.clone(),
            port,
            state: Arc::new(RwLock::new(DeviceState::new(serial_number.clone()))),
            clients: Arc::new(Mutex::new(Vec::new())),
            control_tx,
            handle,
        };

        devices.insert(serial_number.clone(), device);
        print_msg(&format!("Virtual Stream Deck created successfully: Serial='{}', Port={} (Listening on 0.0.0.0:{})", serial_number, port, port));
        Ok(())
    }

    /// Delete a virtual Stream Deck
    async fn delete(&self, serial_number: &str) -> Result<(), Box<dyn std::error::Error>> {
        let mut devices = self.devices.lock().await;
        
        if let Some(device) = devices.remove(serial_number) {
            device.stop();
            print_msg(&format!("Virtual Stream Deck deleted successfully: Serial='{}'", serial_number));
            Ok(())
        } else {
            Err(format!("Device with serial number '{}' not found", serial_number).into())
        }
    }

    /// List all virtual Stream Decks
    async fn list(&self) -> Vec<(String, u16)> {
        let devices = self.devices.lock().await;
        devices.iter().map(|(serial, device)| (serial.clone(), device.port())).collect()
    }

    /// Send control command to a device
    async fn send_control(&self, serial_number: &str, cmd: ControlCommand) -> Result<(), Box<dyn std::error::Error>> {
        let devices = self.devices.lock().await;
        if let Some(device) = devices.get(serial_number) {
            device.send_control(cmd).await?;
            Ok(())
        } else {
            Err(format!("Device with serial number '{}' not found", serial_number).into())
        }
    }
}

/// Parse manager command
#[derive(Debug)]
enum ManagerCommand {
    Create(String),
    Delete(String),
    List,
    Control(String, ControlCommand),
    Help,
    Quit,
}

fn parse_manager_command(line: &str) -> Result<ManagerCommand, String> {
    let parts: Vec<&str> = line.trim().split_whitespace().collect();
    if parts.is_empty() {
        return Err("Empty command".to_string());
    }

    match parts[0].to_lowercase().as_str() {
        "create" => {
            if parts.len() < 2 {
                return Err("Usage: create <serial_number>".to_string());
            }
            Ok(ManagerCommand::Create(parts[1].to_string()))
        }
        "delete" | "remove" => {
            if parts.len() < 2 {
                return Err("Usage: delete <serial_number>".to_string());
            }
            Ok(ManagerCommand::Delete(parts[1].to_string()))
        }
        "list" => Ok(ManagerCommand::List),
        "control" => {
            if parts.len() < 4 {
                return Err("Usage: control <serial> <command> [args...]".to_string());
            }
            let serial = parts[1].to_string();
            let cmd_type = parts[2].to_lowercase();
            match cmd_type.as_str() {
                "button" => {
                    if parts.len() < 5 {
                        return Err("Usage: control <serial> button press/release <index>".to_string());
                    }
                    let action = parts[3].to_lowercase();
                    let index: u8 = parts[4].parse().map_err(|_| "Invalid button index (0-31)".to_string())?;
                    if index >= 32 {
                        return Err("Button index out of range (0-31)".to_string());
                    }
                    let cmd = match action.as_str() {
                        "press" => ControlCommand::ButtonPress(index),
                        "release" => ControlCommand::ButtonRelease(index),
                        _ => return Err("Invalid button action: use 'press' or 'release'".to_string()),
                    };
                    Ok(ManagerCommand::Control(serial, cmd))
                }
                "encoder" => {
                    if parts.len() < 5 {
                        return Err("Usage: control <serial> encoder rotate/press/release <index> [delta]".to_string());
                    }
                    let action = parts[3].to_lowercase();
                    let index: u8 = parts[4].parse().map_err(|_| "Invalid encoder index (0-1)".to_string())?;
                    if index >= 2 {
                        return Err("Encoder index out of range (0-1)".to_string());
                    }
                    let cmd = match action.as_str() {
                        "rotate" => {
                            if parts.len() < 6 {
                                return Err("Usage: control <serial> encoder rotate <index> <delta>".to_string());
                            }
                            let delta: i8 = parts[5].parse().map_err(|_| "Invalid delta (-128 to 127)".to_string())?;
                            ControlCommand::EncoderRotate(index, delta)
                        }
                        "press" => ControlCommand::EncoderPress(index),
                        "release" => ControlCommand::EncoderRelease(index),
                        _ => return Err("Invalid encoder action: use 'rotate', 'press', or 'release'".to_string()),
                    };
                    Ok(ManagerCommand::Control(serial, cmd))
                }
                "touch" => {
                    if parts.len() < 6 {
                        return Err("Usage: control <serial> touch tap/press <x> <y> | touch swipe <x1> <y1> <x2> <y2>".to_string());
                    }
                    let action = parts[3].to_lowercase();
                    match action.as_str() {
                        "tap" | "press" => {
                            let x: u16 = parts[4].parse().map_err(|_| "Invalid x coordinate".to_string())?;
                            let y: u16 = parts[5].parse().map_err(|_| "Invalid y coordinate".to_string())?;
                            let touch_type = if action == "tap" { 0x01 } else { 0x02 };
                            Ok(ManagerCommand::Control(serial, ControlCommand::Touch(touch_type, x, y, None, None)))
                        }
                        "swipe" => {
                            if parts.len() < 8 {
                                return Err("Usage: control <serial> touch swipe <x1> <y1> <x2> <y2>".to_string());
                            }
                            let x1: u16 = parts[4].parse().map_err(|_| "Invalid x1 coordinate".to_string())?;
                            let y1: u16 = parts[5].parse().map_err(|_| "Invalid y1 coordinate".to_string())?;
                            let x2: u16 = parts[6].parse().map_err(|_| "Invalid x2 coordinate".to_string())?;
                            let y2: u16 = parts[7].parse().map_err(|_| "Invalid y2 coordinate".to_string())?;
                            Ok(ManagerCommand::Control(serial, ControlCommand::Touch(0x03, x1, y1, Some(x2), Some(y2))))
                        }
                        _ => Err("Invalid touch action: use 'tap', 'press', or 'swipe'".to_string()),
                    }
                }
                _ => Err(format!("Unknown control command: {}. Use 'button', 'encoder', or 'touch'", cmd_type)),
            }
        }
        "help" => Ok(ManagerCommand::Help),
        "quit" | "exit" => Ok(ManagerCommand::Quit),
        _ => Err(format!("Unknown command: {}. Type 'help' for available commands", parts[0])),
    }
}

/// Print help message
fn print_help() {
    println!("\nStream Deck Manager - Available Commands:");
    println!("  create <serial>           - Create a new virtual Stream Deck with specified serial number");
    println!("  delete <serial>           - Delete a virtual Stream Deck");
    println!("  list                      - List all virtual Stream Decks");
    println!("  control <serial> <cmd>    - Send control command to a device");
    println!("                             Commands:");
    println!("                               button press <index>     - Press button (0-31)");
    println!("                               button release <index>   - Release button (0-31)");
    println!("                               encoder rotate <index> <delta> - Rotate encoder (0-1, delta: -128 to 127)");
    println!("                               encoder press <index>    - Press encoder (0-1)");
    println!("                               encoder release <index>  - Release encoder (0-1)");
    println!("                               touch tap <x> <y>        - Touch tap at coordinates");
    println!("                               touch press <x> <y>      - Touch press at coordinates");
    println!("                               touch swipe <x1> <y1> <x2> <y2> - Swipe from (x1,y1) to (x2,y2)");
    println!("  help                      - Show this help message");
    println!("  quit | exit               - Exit the manager");
    println!();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    println!("Stream Deck Manager - Virtual Stream Deck Studio Manager");
    println!("Type 'help' for available commands");
    println!();

    let manager = Arc::new(StreamDeckManager::new());

    // Handle Ctrl+C
    let manager_signal = manager.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        print_msg("Received Ctrl+C, shutting down...");
        // Stop all devices
        let mut devices = manager_signal.devices.lock().await;
        let device_serials: Vec<String> = devices.keys().cloned().collect();
        for serial in device_serials {
            if let Some(device) = devices.remove(&serial) {
                print_msg(&format!("Stopping device: {}", serial));
                device.stop();
            }
        }
        drop(devices);
        std::process::exit(0);
    });

    // Handle stdin commands
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break,
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                match parse_manager_command(trimmed) {
                    Ok(ManagerCommand::Create(serial)) => {
                        if let Err(e) = manager.create(serial).await {
                            print_error(&format!("Failed to create device: {}", e));
                        }
                    }
                    Ok(ManagerCommand::Delete(serial)) => {
                        if let Err(e) = manager.delete(&serial).await {
                            print_error(&format!("Failed to delete device: {}", e));
                        }
                    }
                    Ok(ManagerCommand::List) => {
                        let devices = manager.list().await;
                        if devices.is_empty() {
                            print_msg("No virtual Stream Decks running");
                        } else {
                            print_msg(&format!("Found {} virtual Stream Deck(s):", devices.len()));
                            for (serial, port) in devices {
                                print_msg(&format!("  Serial: {}, Port: {}", serial, port));
                            }
                        }
                    }
                    Ok(ManagerCommand::Control(serial, cmd)) => {
                        let cmd_clone = cmd.clone();
                        let result = manager.send_control(&serial, cmd).await;
                        match &cmd_clone {
                            ControlCommand::ButtonPress(idx) => {
                                if let Err(e) = result {
                                    print_error(&format!("Failed to send control command: {}", e));
                                } else {
                                    print_msg(&format!("Button {} pressed on {}", idx, serial));
                                }
                            }
                            ControlCommand::ButtonRelease(idx) => {
                                if let Err(e) = result {
                                    print_error(&format!("Failed to send control command: {}", e));
                                } else {
                                    print_msg(&format!("Button {} released on {}", idx, serial));
                                }
                            }
                            ControlCommand::EncoderRotate(idx, delta) => {
                                if let Err(e) = result {
                                    print_error(&format!("Failed to send control command: {}", e));
                                } else {
                                    print_msg(&format!("Encoder {} rotated by {} on {}", idx, delta, serial));
                                }
                            }
                            ControlCommand::EncoderPress(idx) => {
                                if let Err(e) = result {
                                    print_error(&format!("Failed to send control command: {}", e));
                                } else {
                                    print_msg(&format!("Encoder {} pressed on {}", idx, serial));
                                }
                            }
                            ControlCommand::EncoderRelease(idx) => {
                                if let Err(e) = result {
                                    print_error(&format!("Failed to send control command: {}", e));
                                } else {
                                    print_msg(&format!("Encoder {} released on {}", idx, serial));
                                }
                            }
                            ControlCommand::Touch(tt, x, y, ex, ey) => {
                                if let Err(e) = result {
                                    print_error(&format!("Failed to send control command: {}", e));
                                } else {
                                    let touch_name = match *tt {
                                        0x01 => "tap",
                                        0x02 => "press",
                                        0x03 => "swipe",
                                        _ => "unknown",
                                    };
                                    if *tt == 0x03 {
                                        print_msg(&format!("Touch {} from ({},{}) to ({},{}) on {}", 
                                            touch_name, x, y, ex.unwrap_or(0), ey.unwrap_or(0), serial));
                                    } else {
                                        print_msg(&format!("Touch {} at ({},{}) on {}", touch_name, x, y, serial));
                                    }
                                }
                            }
                        }
                    }
                    Ok(ManagerCommand::Help) => {
                        print_help();
                    }
                    Ok(ManagerCommand::Quit) => {
                        print_msg("Shutting down...");
                        // Stop all devices
                        let mut devices = manager.devices.lock().await;
                        let device_serials: Vec<String> = devices.keys().cloned().collect();
                        for serial in device_serials {
                            if let Some(device) = devices.remove(&serial) {
                                print_msg(&format!("Stopping device: {}", serial));
                                device.stop();
                            }
                        }
                        drop(devices);
                        break;
                    }
                    Err(e) => {
                        print_error(&format!("{}", e));
                    }
                }
            }
            Err(e) => {
                print_error(&format!("Failed to read from stdin: {}", e));
                break;
            }
        }
    }

    Ok(())
}