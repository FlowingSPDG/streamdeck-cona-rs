//! Interactive CUI client for Stream Deck Studio
//!
//! This is a CUI tool that provides an interactive command-line interface for controlling
//! a Stream Deck Studio device over TCP.
//!
//! Usage: streamdeck-client [IP:PORT]

use streamdeck_cora_rs::Device;
use std::io::{self, Write};
use tokio::io::{AsyncBufReadExt, BufReader};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // Get IP address from command line or use default
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "192.168.1.100:5343".to_string());

    println!("Connecting to Stream Deck Studio at {}...", addr);

    let device = match Device::connect_tcp(&addr).await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to connect: {}", e);
            eprintln!("\nUsage: {} [IP:PORT]", std::env::args().next().unwrap());
            eprintln!("Example: {} 192.168.1.100:5343", std::env::args().next().unwrap());
            return Err(e.into());
        }
    };

    // Get serial number
    if let Some(serial) = device.serial_number().await {
        println!("✓ Connected! Serial: {}", serial);
    } else {
        println!("✓ Connected!");
    }

    println!("\nAvailable commands:");
    println!("  brightness <0-100>  - Set brightness");
    println!("  button <0-31> <image_path>  - Set button image");
    println!("  encoder <0-1> <r> <g> <b>  - Set encoder LED color");
    println!("  ring <0-1> <colors...>  - Set encoder ring (24 colors as r,g,b r,g,b ...)");
    println!("  reset  - Reset device");
    println!("  events - Enable event monitoring");
    println!("  help   - Show this help");
    println!("  quit   - Exit");
    println!();

    // Start event monitoring task (placeholder)
    let device_clone = std::sync::Arc::new(tokio::sync::Mutex::new(device));

    // Command input loop
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();

    loop {
        print!("streamdeck> ");
        io::stdout().flush()?;
        line.clear();

        if reader.read_line(&mut line).await? == 0 {
            break; // EOF
        }

        let parts: Vec<&str> = line.trim().split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        match parts[0] {
            "quit" | "exit" | "q" => {
                println!("Goodbye!");
                break;
            }

            "help" | "h" => {
                println!("Commands:");
                println!("  brightness <0-100>");
                println!("  button <0-31> <image_path>");
                println!("  encoder <0-1> <r> <g> <b>");
                println!("  ring <0-1> <colors...>");
                println!("  reset");
                println!("  events");
                println!("  help");
                println!("  quit");
            }

            "brightness" => {
                if parts.len() < 2 {
                    println!("Usage: brightness <0-100>");
                    continue;
                }
                match parts[1].parse::<u8>() {
                    Ok(b) if b <= 100 => {
                        let device = device_clone.lock().await;
                        match device.set_brightness(b).await {
                            Ok(_) => println!("✓ Brightness set to {}%", b),
                            Err(e) => eprintln!("✗ Error: {}", e),
                        }
                    }
                    _ => println!("Brightness must be 0-100"),
                }
            }

            "button" => {
                if parts.len() < 3 {
                    println!("Usage: button <0-31> <image_path>");
                    continue;
                }
                match parts[1].parse::<u8>() {
                    Ok(btn) if btn < 32 => {
                        let image_path = parts[2];
                        match std::fs::read(image_path) {
                            Ok(data) => {
                                let device = device_clone.lock().await;
                                match device.set_button_image(btn, data).await {
                                    Ok(_) => println!("✓ Button {} image set", btn),
                                    Err(e) => eprintln!("✗ Error: {}", e),
                                }
                            }
                            Err(e) => eprintln!("✗ Failed to read image: {}", e),
                        }
                    }
                    _ => println!("Button index must be 0-31"),
                }
            }

            "encoder" => {
                if parts.len() < 5 {
                    println!("Usage: encoder <0-1> <r> <g> <b>");
                    continue;
                }
                match (
                    parts[1].parse::<u8>(),
                    parts[2].parse::<u8>(),
                    parts[3].parse::<u8>(),
                    parts[4].parse::<u8>(),
                ) {
                    (Ok(enc), Ok(r), Ok(g), Ok(b)) if enc < 2 => {
                        let device = device_clone.lock().await;
                        match device.set_encoder_color(enc, r, g, b).await {
                            Ok(_) => println!("✓ Encoder {} color set to RGB({},{},{})", enc, r, g, b),
                            Err(e) => eprintln!("✗ Error: {}", e),
                        }
                    }
                    _ => println!("Usage: encoder <0-1> <r> <g> <b> (all 0-255)"),
                }
            }

            "ring" => {
                if parts.len() < 3 {
                    println!("Usage: ring <0-1> <colors...>");
                    println!("  Colors format: r,g,b r,g,b ... (up to 24 colors)");
                    continue;
                }
                match parts[1].parse::<u8>() {
                    Ok(enc) if enc < 2 => {
                        let mut colors = Vec::new();
                        for color_str in parts.iter().skip(2) {
                            let rgb: Vec<&str> = color_str.split(',').collect();
                            if rgb.len() == 3 {
                                match (
                                    rgb[0].parse::<u8>(),
                                    rgb[1].parse::<u8>(),
                                    rgb[2].parse::<u8>(),
                                ) {
                                    (Ok(r), Ok(g), Ok(b)) => colors.push((r, g, b)),
                                    _ => {
                                        println!("Invalid color format: {}", color_str);
                                        continue;
                                    }
                                }
                            } else {
                                println!("Invalid color format: {}", color_str);
                                continue;
                            }
                            if colors.len() >= 24 {
                                break;
                            }
                        }
                        if !colors.is_empty() {
                            let device = device_clone.lock().await;
                            match device.set_encoder_ring(enc, colors.clone()).await {
                                Ok(_) => println!("✓ Encoder {} ring set with {} colors", enc, colors.len()),
                                Err(e) => eprintln!("✗ Error: {}", e),
                            }
                        } else {
                            println!("No valid colors provided");
                        }
                    }
                    _ => println!("Encoder index must be 0 or 1"),
                }
            }

            "reset" => {
                let device = device_clone.lock().await;
                match device.reset().await {
                    Ok(_) => println!("✓ Device reset"),
                    Err(e) => eprintln!("✗ Error: {}", e),
                }
            }

            "events" => {
                println!("Event monitoring (placeholder)");
                // In a real implementation, this would show events as they arrive
            }

            "" => continue,

            cmd => {
                println!("Unknown command: {}. Type 'help' for available commands.", cmd);
            }
        }
    }

    Ok(())
}
