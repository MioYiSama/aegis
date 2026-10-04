use aegis_core::qr::{QrError, decode_chroma, decode_chroma_region, encode_chroma};
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

#[test]
fn exposed_ring_is_detected_against_a_dark_camera_background() {
    let mut carrier = encode_chroma(TOKEN).unwrap();
    // A thin screen border can be washed out by the camera while the colored
    // payload stays darker. The white gap, not the payload, identifies it.
    for pixel in carrier.pixels_mut() {
        if pixel.0 == [0, 0, 0] {
            *pixel = Rgb([175, 175, 175]);
        }
    }
    for side in [420, 300] {
        let resized = imageops::resize(&carrier, side, side, imageops::FilterType::Triangle);
        let mut photo = RgbImage::from_pixel(720, 1280, Rgb([35, 35, 35]));
        imageops::replace(&mut photo, &resized, 100, 320);
        assert_eq!(decode_chroma(&photo).unwrap(), TOKEN, "carrier side {side}");
    }
}

#[test]
fn damaged_finders_candidate_does_not_hide_a_readable_carrier() {
    let mut damaged = encode_chroma(TOKEN).unwrap();
    // Keep all three finders, but destroy the format and data modules.
    for row in 0..29 {
        for col in 0..29 {
            if (row < 7 && (col < 7 || col >= 22)) || (row >= 22 && col < 7) {
                continue;
            }
            for y in 0..16 {
                for x in 0..16 {
                    let color = if (x / 4 + y / 4) & 1 == 0 {
                        [210, 74, 120]
                    } else {
                        [30, 166, 120]
                    };
                    damaged.put_pixel(32 + (col + 4) * 16 + x, 32 + (row + 4) * 16 + y, Rgb(color));
                }
            }
        }
    }
    assert_eq!(decode_chroma(&damaged), Err(QrError::Unreadable));
    let damaged = imageops::resize(&damaged, 420, 420, imageops::FilterType::Triangle);
    let readable = imageops::resize(
        &encode_chroma(TOKEN).unwrap(),
        300,
        300,
        imageops::FilterType::Triangle,
    );
    let mut photo = RgbImage::from_pixel(720, 1280, Rgb([238, 238, 238]));
    imageops::replace(&mut photo, &damaged, 100, 100);
    imageops::replace(&mut photo, &readable, 160, 700);
    assert_eq!(decode_chroma(&photo).unwrap(), TOKEN);
}

#[test]
fn cropped_camera_pixels_remain_decodable_including_at_image_edges() {
    let carrier = encode_chroma(TOKEN).unwrap();
    for (x, y) in [(0, 0), (100, 320), (424, 1264)] {
        let mut photo = RgbImage::from_pixel(1080, 1920, Rgb([35, 35, 35]));
        imageops::replace(&mut photo, &carrier, x, y);
        let (_, [left, top, width, height]) = decode_chroma_region(&photo).unwrap();
        let cropped = imageops::crop_imm(&photo, left, top, width, height).to_image();
        assert_eq!(
            decode_chroma(&cropped).unwrap(),
            TOKEN,
            "carrier at ({x}, {y})"
        );
    }
}

#[test]
fn real_screen_photo_and_upload_crop_recover_the_token() {
    let photo = image::load_from_memory(include_bytes!("fixtures/screen-carrier.jpg"))
        .unwrap()
        .into_rgb8();
    let expected = "ZDU3LAPUqzBr8Efbh1p0cRKhCZuVLOFf";
    let (payload, [left, top, width, height]) = decode_chroma_region(&photo).unwrap();
    assert_eq!(payload, expected);
    let cropped = imageops::crop_imm(&photo, left, top, width, height).to_image();
    assert_eq!(decode_chroma(&cropped).unwrap(), expected);
    // The client's 9:16 preview discards only the sides of this 4:3 photo.
    let preview = imageops::crop_imm(&photo, 160, 0, 960, photo.height()).to_image();
    assert_eq!(decode_chroma(&preview).unwrap(), expected);
    for rotated in [
        imageops::rotate90(&photo),
        imageops::rotate180(&photo),
        imageops::rotate270(&photo),
    ] {
        assert_eq!(decode_chroma(&rotated).unwrap(), expected);
    }
}
