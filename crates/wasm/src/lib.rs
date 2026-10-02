use aegis_core::qr::{decode_chroma, encode_chroma, QrError};
use image::{DynamicImage, ImageFormat, RgbImage};
use std::io::Cursor;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn encode_chroma_png(payload: &str) -> Result<Vec<u8>, JsError> {
    let image = encode_chroma(payload).map_err(|_| JsError::new("invalid_payload"))?;
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(image).write_to(&mut bytes, ImageFormat::Png)
        .map_err(|_| JsError::new("encode_failed"))?;
    Ok(bytes.into_inner())
}

#[wasm_bindgen]
pub fn decode_chroma_rgba(rgba: &[u8], width: u32, height: u32) -> Result<Option<String>, JsError> {
    let length = width.checked_mul(height).and_then(|pixels| pixels.checked_mul(4));
    if !(1..=1920).contains(&width) || !(1..=1920).contains(&height)
        || length.map(|n| n as usize) != Some(rgba.len()) {
        return Err(JsError::new("invalid_image"));
    }
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for pixel in rgba.chunks_exact(4) { rgb.extend_from_slice(&pixel[..3]); }
    let image = RgbImage::from_raw(width, height, rgb).ok_or_else(|| JsError::new("invalid_image"))?;
    match decode_chroma(&image) {
        Ok(payload) => Ok(Some(payload)),
        Err(QrError::Unreadable) => Ok(None),
        Err(_) => Err(JsError::new("decode_failed")),
    }
}
