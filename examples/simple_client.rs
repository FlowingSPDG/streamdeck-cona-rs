//! Simple client example for Stream Deck Studio

use streamdeck_rs_tcp::Device;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // Connect to Stream Deck Studio
    // Replace with your device's IP address
    let _device = Device::connect_tcp("192.168.1.100:5343").await?;

    // Get and print serial number
    if let Some(serial) = _device.serial_number().await {
        println!("Connected to device with serial: {}", serial);
    }

    // Set brightness to 80%
    println!("Setting brightness to 80%...");
    _device.set_brightness(80).await?;

    // Reset device
    // println!("Resetting device...");
    // device.reset().await?;

    println!("Done!");

    // Keep the connection alive
    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

    Ok(())
}
