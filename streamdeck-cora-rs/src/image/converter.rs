//! Image conversion utilities for Stream Deck Studio

use crate::error::{Error, Result};
use image::ImageFormat;

/// Stream Deck Studio button image dimensions
pub const BUTTON_WIDTH: u32 = 144;
pub const BUTTON_HEIGHT: u32 = 112;

/// Stream Deck Studio display area dimensions
pub const DISPLAY_WIDTH: u32 = 800;
pub const DISPLAY_HEIGHT: u32 = 100;

/// Image converter for Stream Deck Studio
pub struct ImageConverter;

impl ImageConverter {
    /// Convert an image to JPEG format suitable for Stream Deck Studio button
    ///
    /// # Arguments
    /// * `image_data` - Input image data (any format supported by image crate)
    /// * `width` - Target width (default: 144 for buttons)
    /// * `height` - Target height (default: 112 for buttons)
    ///
    /// # Returns
    /// JPEG-encoded image data as Vec<u8>
    pub fn convert_to_jpeg(
        image_data: &[u8],
        width: Option<u32>,
        height: Option<u32>,
    ) -> Result<Vec<u8>> {
        let width = width.unwrap_or(BUTTON_WIDTH);
        let height = height.unwrap_or(BUTTON_HEIGHT);

        // Load image
        let img = image::load_from_memory(image_data)
            .map_err(|e| Error::Image(format!("Failed to load image: {}", e)))?;

        // Resize image
        let resized = img.resize_exact(
            width,
            height,
            image::imageops::FilterType::Lanczos3,
        );

        // Encode as JPEG
        let mut jpeg_data = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut jpeg_data);
        resized
            .write_to(&mut cursor, ImageFormat::Jpeg)
            .map_err(|e| Error::Image(format!("Failed to encode JPEG: {}", e)))?;

        Ok(jpeg_data)
    }

    /// Convert an image for button display (144x112)
    pub fn convert_button_image(image_data: &[u8]) -> Result<Vec<u8>> {
        Self::convert_to_jpeg(image_data, Some(BUTTON_WIDTH), Some(BUTTON_HEIGHT))
    }

    /// Convert an image for display area
    pub fn convert_display_area_image(
        image_data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>> {
        Self::convert_to_jpeg(image_data, Some(width), Some(height))
    }

    /// Split image data into chunks for multi-packet transmission
    ///
    /// For button images: payload per packet is 1024 - 8 = 1016 bytes
    /// For display area images: payload per packet is 1024 - 16 = 1008 bytes
    pub fn split_button_image_data(data: &[u8]) -> Vec<&[u8]> {
        const PAYLOAD_SIZE: usize = 1016; // 1024 - 8 byte header
        let mut chunks = Vec::new();
        let mut offset = 0;

        while offset < data.len() {
            let end = (offset + PAYLOAD_SIZE).min(data.len());
            chunks.push(&data[offset..end]);
            offset = end;
        }

        chunks
    }

    /// Split image data into chunks for display area multi-packet transmission
    pub fn split_display_area_image_data(data: &[u8]) -> Vec<&[u8]> {
        const PAYLOAD_SIZE: usize = 1008; // 1024 - 16 byte header
        let mut chunks = Vec::new();
        let mut offset = 0;

        while offset < data.len() {
            let end = (offset + PAYLOAD_SIZE).min(data.len());
            chunks.push(&data[offset..end]);
            offset = end;
        }

        chunks
    }
}
