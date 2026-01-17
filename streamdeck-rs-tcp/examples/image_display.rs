//! Image display example for Stream Deck Studio

use streamdeck_rs_tcp::Device;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // Connect to Stream Deck Studio
    // Replace with your device's IP address
    let device = Device::connect_tcp("192.168.1.100:5343").await?;

    println!("Connected to device");

    // Example: Set a solid color image on button 5
    // In practice, you would load an actual image file
    // For this example, we'll create a simple red image
    println!("Setting button 5 image...");
    
    // Create a simple 144x112 red image
    let image_data = create_simple_red_image()?;
    device.set_button_image(5, image_data).await?;

    println!("Image set on button 5");

    // Keep the connection alive to see the image
    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

    Ok(())
}

fn create_simple_red_image() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use image::{ImageBuffer, Rgb, RgbImage};

    // Create a 144x112 red image
    let img: RgbImage = ImageBuffer::from_fn(144, 112, |_, _| Rgb([255u8, 0u8, 0u8]));

    // Encode as JPEG
    let mut jpeg_data = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut jpeg_data);
    img.write_to(&mut cursor, image::ImageFormat::Jpeg)?;

    Ok(jpeg_data)
}
