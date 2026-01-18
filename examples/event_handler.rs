//! Event handler example for Stream Deck Studio

use streamdeck_cona_rs::Device;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // Connect to Stream Deck Studio
    // Replace with your device's IP address
    let device = Device::connect_tcp("192.168.1.100:5343").await?;

    println!("Connected to device");
    println!("Press buttons, rotate encoders, or touch the screen...");
    println!("Press Ctrl+C to exit");

    // Note: Event handling is implemented but callback API is not yet complete
    // For now, the event loop is running in the background
    // Future versions will provide a callback-based API

    // Keep the program running
    tokio::signal::ctrl_c().await?;
    println!("\nExiting...");

    Ok(())
}
