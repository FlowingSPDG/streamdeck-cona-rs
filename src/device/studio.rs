//! Stream Deck Studio device implementation

use crate::connection::TcpConnection;
use crate::error::{Error, Result};
use crate::image::ImageConverter;
use crate::protocol::command::{Command, CommandEncoder};
use crate::protocol::event::{Event, EventDecoder};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::RwLock;

/// Stream Deck Studio device
pub struct Device {
    connection: Arc<RwLock<TcpConnection>>,
    event_sender: Option<mpsc::UnboundedSender<Event>>,
}

impl Device {
    /// Connect to a Stream Deck Studio device via TCP
    ///
    /// # Arguments
    /// * `addr` - Address in format "IP:port" (e.g., "192.168.1.100:5343")
    ///
    /// # Example
    /// ```no_run
    /// use streamdeck_rs_tcp::Device;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let device = Device::connect_tcp("192.168.1.100:5343").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn connect_tcp(addr: impl AsRef<str>) -> Result<Self> {
        let connection = Arc::new(RwLock::new(TcpConnection::connect(addr).await?));

        let device = Self {
            connection,
            event_sender: None,
        };

        // Start event handling loop
        device.start_event_loop();

        Ok(device)
    }

    /// Get the serial number of the device
    pub async fn serial_number(&self) -> Option<String> {
        self.connection.read().await.serial_number().await
    }

    /// Set the brightness of all buttons
    ///
    /// # Arguments
    /// * `brightness` - Brightness value from 0 to 100
    pub async fn set_brightness(&self, brightness: u8) -> Result<()> {
        if brightness > 100 {
            return Err(Error::InvalidParameter(format!(
                "Brightness must be 0-100, got {}",
                brightness
            )));
        }

        let command = Command::SetBrightness(brightness);
        let packet = CommandEncoder::encode(&command)?;
        self.connection.read().await.send_packet(&packet).await?;
        Ok(())
    }

    /// Set an image on a button
    ///
    /// # Arguments
    /// * `button_index` - Button index (0-31)
    /// * `image_data` - Image data (JPEG format, will be converted if needed)
    ///
    /// # Example
    /// ```no_run
    /// # use streamdeck_rs_tcp::Device;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let device = Device::connect_tcp("192.168.1.100:5343").await?;
    /// let image_data = include_bytes!("button.jpg");
    /// device.set_button_image(5, image_data.to_vec()).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_button_image(
        &self,
        button_index: u8,
        image_data: Vec<u8>,
    ) -> Result<()> {
        if button_index >= 32 {
            return Err(Error::InvalidParameter(format!(
                "Button index must be 0-31, got {}",
                button_index
            )));
        }

        // Convert image to JPEG if needed and resize to 144x112
        let jpeg_data = ImageConverter::convert_button_image(&image_data)?;

        // Split into chunks for multi-packet transmission
        let chunks = ImageConverter::split_button_image_data(&jpeg_data);

        for (page, chunk) in chunks.iter().enumerate() {
            let is_last_page = page == chunks.len() - 1;
            let command = Command::SetButtonImage {
                button_index,
                is_last_page,
                page_number: page as u16,
                data: chunk.to_vec(),
            };
            let packet = CommandEncoder::encode(&command)?;
            self.connection.read().await.send_packet(&packet).await?;
        }

        Ok(())
    }

    /// Set an image on the display area (encoder screen)
    ///
    /// # Arguments
    /// * `x` - X coordinate (0, 200, 400, or 600)
    /// * `y` - Y coordinate (usually 0)
    /// * `width` - Image width
    /// * `height` - Image height
    /// * `image_data` - Image data (JPEG format, will be converted if needed)
    pub async fn set_display_area_image(
        &self,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        image_data: Vec<u8>,
    ) -> Result<()> {
        // Convert image to JPEG if needed and resize
        let jpeg_data = ImageConverter::convert_display_area_image(&image_data, width as u32, height as u32)?;

        // Split into chunks for multi-packet transmission
        let chunks = ImageConverter::split_display_area_image_data(&jpeg_data);

        for (page, chunk) in chunks.iter().enumerate() {
            let is_last_page = page == chunks.len() - 1;
            let command = Command::SetDisplayAreaImage {
                x,
                y,
                width,
                height,
                is_last_page,
                page_number: page as u16,
                data: chunk.to_vec(),
            };
            let packet = CommandEncoder::encode(&command)?;
            self.connection.read().await.send_packet(&packet).await?;
        }

        Ok(())
    }

    /// Set the color of an encoder LED
    ///
    /// # Arguments
    /// * `encoder_index` - Encoder index (0 or 1)
    /// * `r` - Red component (0-255)
    /// * `g` - Green component (0-255)
    /// * `b` - Blue component (0-255)
    pub async fn set_encoder_color(
        &self,
        encoder_index: u8,
        r: u8,
        g: u8,
        b: u8,
    ) -> Result<()> {
        if encoder_index >= 2 {
            return Err(Error::InvalidParameter(format!(
                "Encoder index must be 0 or 1, got {}",
                encoder_index
            )));
        }

        let command = Command::SetEncoderColor {
            encoder_index,
            r,
            g,
            b,
        };
        let packet = CommandEncoder::encode(&command)?;
        self.connection.read().await.send_packet(&packet).await?;
        Ok(())
    }

    /// Set the colors of an encoder ring LED (24 segments)
    ///
    /// # Arguments
    /// * `encoder_index` - Encoder index (0 or 1)
    /// * `colors` - Vector of RGB tuples (up to 24 colors)
    pub async fn set_encoder_ring(
        &self,
        encoder_index: u8,
        colors: Vec<(u8, u8, u8)>,
    ) -> Result<()> {
        if encoder_index >= 2 {
            return Err(Error::InvalidParameter(format!(
                "Encoder index must be 0 or 1, got {}",
                encoder_index
            )));
        }

        if colors.len() > 24 {
            return Err(Error::InvalidParameter(format!(
                "Too many colors: max 24, got {}",
                colors.len()
            )));
        }

        let command = Command::SetEncoderRing {
            encoder_index,
            colors,
        };
        let packet = CommandEncoder::encode(&command)?;
        self.connection.read().await.send_packet(&packet).await?;
        Ok(())
    }

    /// Reset the device
    pub async fn reset(&self) -> Result<()> {
        let command = Command::Reset;
        let packet = CommandEncoder::encode(&command)?;
        self.connection.read().await.send_packet(&packet).await?;
        Ok(())
    }

    /// Start the event handling loop
    fn start_event_loop(&self) {
        let connection = Arc::clone(&self.connection);
        let (tx, _rx) = mpsc::unbounded_channel::<Event>();

        // Event receiver task
        let event_loop_connection = Arc::clone(&connection);
        tokio::spawn(async move {
            loop {
                match event_loop_connection.read().await.read_packet().await {
                    Ok(packet) => {
                        // Check if this is a keep-alive request
                        if packet[0] == 0x01 && packet[1] == 0x0a {
                            if let Err(e) = event_loop_connection.read().await.handle_keep_alive().await {
                                log::error!("Failed to send keep-alive response: {}", e);
                            }
                            continue;
                        }

                        // Decode event
                        match EventDecoder::decode(&packet) {
                            Ok(Some(event)) => {
                                if let Err(_) = tx.send(event) {
                                    // Receiver dropped, exit loop
                                    break;
                                }
                            }
                            Ok(None) => {
                                // Not an event packet, ignore
                            }
                            Err(e) => {
                                log::debug!("Failed to decode event: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to read packet: {}", e);
                        // Exit loop on error
                        break;
                    }
                }
            }
        });

        // Store sender in device (simplified - in practice we might need better state management)
        // For now, we'll handle events through a callback-based API
    }

    /// Handle events with a callback (simplified implementation)
    /// 
    /// Note: This is a placeholder for future event handling API
    pub async fn on_event<F>(&self, _callback: F)
    where
        F: Fn(Event) + Send + 'static,
    {
        // TODO: Implement callback-based event handling
        // This requires storing callbacks and routing events to them
        log::warn!("Event callbacks not yet implemented");
    }
}
