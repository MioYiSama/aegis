use aegis_core::qr::{QrError, decode_chroma, encode_chroma};
use image::{GrayImage, Luma, Rgb, RgbImage, imageops};
use imageproc::geometric_transformations::{Interpolation, Projection, warp};
use qrcode::{Color as QrColor, EcLevel, QrCode, Version};

const TOKEN: &str = "Abcdefghijklmnopqrstuvwxyz012345";

#[test]
fn fixed_carrier_round_trips_token_text() {
    let carrier = encode_chroma(TOKEN).expect("valid 32-character token");
    assert_eq!(carrier.dimensions(), (656, 656));
    assert_eq!(decode_chroma(&carrier).unwrap(), TOKEN);
}

#[test]
fn rotated_and_perspective_carriers_round_trip() {
    let carrier = encode_chroma(TOKEN).unwrap();
    let rotated = imageops::rotate90(&carrier);
    assert_eq!(decode_chroma(&rotated).unwrap(), TOKEN);

    let last = carrier.width() as f32 - 1.0;
    let from = [(0.0, 0.0), (last, 0.0), (last, last), (0.0, last)];
    let to = [
        (28.0, 15.0),
        (last - 31.0, 37.0),
        (last - 14.0, last - 25.0),
        (19.0, last - 8.0),
    ];
    let projection = Projection::from_control_points(from, to).expect("valid perspective");
    let perspective = warp(
        &carrier,
        &projection,
        Interpolation::Bilinear,
        Rgb([255, 255, 255]),
    );
    assert_eq!(decode_chroma(&perspective).unwrap(), TOKEN);
}

#[test]
fn blank_and_monochrome_qr_are_unreadable() {
    let blank = RgbImage::from_pixel(656, 656, Rgb([255, 255, 255]));
    assert_eq!(decode_chroma(&blank), Err(QrError::Unreadable));

    let code = QrCode::with_version(TOKEN.as_bytes(), Version::Normal(3), EcLevel::M).unwrap();
    let modules = 29_usize + 8;
    let scale = 8_usize;
    let mut grayscale = GrayImage::from_pixel(
        (modules * scale) as u32,
        (modules * scale) as u32,
        Luma([255]),
    );
    for row in 0..29 {
        for col in 0..29 {
            if code[(col, row)] == QrColor::Dark {
                for y in (row + 4) * scale..(row + 5) * scale {
                    for x in (col + 4) * scale..(col + 5) * scale {
                        grayscale.put_pixel(x as u32, y as u32, Luma([0]));
                    }
                }
            }
        }
    }
    let monochrome = RgbImage::from_fn(grayscale.width(), grayscale.height(), |x, y| {
        let luminance = grayscale.get_pixel(x, y).0[0];
        Rgb([luminance, luminance, luminance])
    });
    assert_eq!(decode_chroma(&monochrome), Err(QrError::Unreadable));
}

#[test]
fn rejects_payloads_outside_the_fixed_ascii_contract() {
    assert_eq!(encode_chroma(&"a".repeat(43)), Err(QrError::InvalidPayload));
    assert_eq!(encode_chroma(""), Err(QrError::InvalidPayload));
    assert_eq!(encode_chroma("token-雪"), Err(QrError::InvalidPayload));
}
