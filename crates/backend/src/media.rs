use crate::error::{ApiError, ApiResult};
use axum::{
    extract::{FromRequest, Request},
    http::StatusCode,
};
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbImage};
use std::{collections::HashMap, io::Cursor};

pub const MULTIPART_LIMIT: usize = 12 * 1024 * 1024;
pub const IMAGE_LIMIT: usize = 2 * 1024 * 1024;
pub const TEXT_LIMIT: usize = 16 * 1024;
pub const EVIDENCE_LIMIT: usize = 5 * 1024 * 1024;
pub struct Multipart(pub axum::extract::Multipart);
impl<S: Send + Sync> FromRequest<S> for Multipart {
    type Rejection = ApiError;
    async fn from_request(req: Request, state: &S) -> ApiResult<Self> {
        axum::extract::Multipart::from_request(req, state)
            .await
            .map(Self)
            .map_err(|error| {
                if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    ApiError::too_large()
                } else {
                    ApiError::invalid("Invalid multipart request")
                }
            })
    }
}
#[derive(Clone, Copy)]
pub enum FieldKind {
    Text,
    Image,
    Evidence,
}
pub struct FieldRule {
    pub name: &'static str,
    pub kind: FieldKind,
}
pub struct MediaField {
    pub bytes: Vec<u8>,
    pub mime: Option<String>,
}
impl MediaField {
    pub fn text(&self) -> ApiResult<&str> {
        std::str::from_utf8(&self.bytes).map_err(|_| ApiError::invalid("Text must be UTF-8"))
    }
}
fn multipart_error(error: axum::extract::multipart::MultipartError) -> ApiError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ApiError::too_large()
    } else {
        ApiError::invalid("Invalid multipart field")
    }
}
pub async fn receive(
    mut multipart: Multipart,
    rules: &[FieldRule],
) -> ApiResult<HashMap<String, MediaField>> {
    let mut fields = HashMap::with_capacity(rules.len());
    let mut total = 0usize;
    while let Some(mut field) = multipart.0.next_field().await.map_err(multipart_error)? {
        let name = field
            .name()
            .ok_or_else(|| ApiError::invalid("Named fields required"))?
            .to_owned();
        let rule = rules
            .iter()
            .find(|rule| rule.name == name)
            .ok_or_else(|| ApiError::invalid("Unknown multipart field"))?;
        if fields.contains_key(&name) {
            return Err(ApiError::invalid("Duplicate multipart field"));
        }
        let mime = field.content_type().map(str::to_owned);
        match rule.kind {
            FieldKind::Image if !matches!(mime.as_deref(), Some("image/png" | "image/jpeg")) => {
                return Err(ApiError::invalid("Images must be JPEG or PNG"));
            }
            FieldKind::Evidence
                if !matches!(
                    mime.as_deref(),
                    Some("image/png" | "image/jpeg" | "application/pdf")
                ) =>
            {
                return Err(ApiError::invalid("Unsupported evidence type"));
            }
            FieldKind::Text
                if mime
                    .as_deref()
                    .is_some_and(|m| !matches!(m, "text/plain" | "application/json")) =>
            {
                return Err(ApiError::invalid("Invalid text field type"));
            }
            _ => {}
        }
        let limit = match rule.kind {
            FieldKind::Text => TEXT_LIMIT,
            FieldKind::Image => IMAGE_LIMIT,
            FieldKind::Evidence => EVIDENCE_LIMIT,
        };
        let mut bytes = Vec::new();
        while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
            total = total
                .checked_add(chunk.len())
                .ok_or_else(ApiError::too_large)?;
            if total > MULTIPART_LIMIT || bytes.len().saturating_add(chunk.len()) > limit {
                return Err(ApiError::too_large());
            }
            bytes.extend_from_slice(&chunk);
        }
        if matches!(rule.kind, FieldKind::Evidence) {
            validate_evidence(&bytes, mime.as_deref().unwrap_or(""))?;
        }
        fields.insert(name, MediaField { bytes, mime });
    }
    Ok(fields)
}
pub fn validate_evidence(bytes: &[u8], mime: &str) -> ApiResult<()> {
    let valid = match mime {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(b"\xff\xd8\xff"),
        "application/pdf" => bytes.starts_with(b"%PDF-"),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(ApiError::invalid(
            "Evidence magic does not match media type",
        ))
    }
}
pub async fn decode_image(field: MediaField) -> ApiResult<RgbImage> {
    tokio::task::spawn_blocking(move || {
        if field.bytes.len() > IMAGE_LIMIT {
            return Err(ApiError::too_large());
        }
        let mut reader = ImageReader::new(Cursor::new(&field.bytes))
            .with_guessed_format()
            .map_err(|_| ApiError::invalid("Unreadable image"))?;
        let format = reader
            .format()
            .ok_or_else(|| ApiError::invalid("Unknown image format"))?;
        let expected = match field.mime.as_deref() {
            Some("image/png") => ImageFormat::Png,
            Some("image/jpeg") => ImageFormat::Jpeg,
            _ => return Err(ApiError::invalid("Images must be JPEG or PNG")),
        };
        if format != expected {
            return Err(ApiError::invalid("Image magic does not match media type"));
        }
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(1920);
        limits.max_image_height = Some(1920);
        limits.max_alloc = Some(64 * 1024 * 1024);
        reader.limits(limits);
        let mut decoder = reader
            .into_decoder()
            .map_err(|_| ApiError::invalid("Image dimensions or encoding invalid"))?;
        let (w, h) = decoder.dimensions();
        if w == 0 || h == 0 || w > 1920 || h > 1920 || decoder.total_bytes() > 64 * 1024 * 1024 {
            return Err(ApiError::invalid("Image dimensions exceed limit"));
        }
        let orientation = decoder
            .orientation()
            .map_err(|_| ApiError::invalid("Invalid image metadata"))?;
        let mut image = DynamicImage::from_decoder(decoder)
            .map_err(|_| ApiError::invalid("Unreadable image"))?;
        image.apply_orientation(orientation);
        Ok(image.into_rgb8())
    })
    .await
    .map_err(|_| ApiError::internal())?
}
