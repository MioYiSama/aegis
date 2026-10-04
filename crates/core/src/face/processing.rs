use image::RgbImage;
use tract_onnx::prelude::{TValue, Tensor};

use super::FaceError;

const DETECTOR_STRIDES: [usize; 3] = [8, 16, 32];
const DETECTOR_OUTPUT_COUNT: usize = 12;
const DETECTOR_SCORE_THRESHOLD: f32 = 0.9;
const DETECTOR_NMS_THRESHOLD: f64 = 0.3;
const DETECTOR_TOP_K: usize = 5000;
const LIVE_SIZE: usize = 80;
const FACE_SIZE: usize = 112;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Detection {
    pub bbox: [f32; 4],
    pub landmarks: [[f32; 2]; 5],
}

pub(super) fn detector_input(frame: &RgbImage) -> Result<Tensor, FaceError> {
    let width = frame.width() as usize;
    let height = frame.height() as usize;
    if width == 0 || height == 0 {
        return Err(FaceError::NotSingleFace);
    }

    let padded_width = pad_to_32(width)?;
    let padded_height = pad_to_32(height)?;
    let plane_size = padded_width
        .checked_mul(padded_height)
        .ok_or(FaceError::Inference)?;
    let tensor_size = plane_size.checked_mul(3).ok_or(FaceError::Inference)?;
    let expected_input_size = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or(FaceError::Inference)?;
    if frame.as_raw().len() != expected_input_size {
        return Err(FaceError::Inference);
    }

    // Zero-initialization supplies YuNet's bottom/right black padding directly in
    // the final NCHW allocation.
    let mut input = Tensor::zero::<f32>(&[1, 3, padded_height, padded_width])
        .map_err(|_| FaceError::Inference)?;
    let mut storage = input
        .try_as_plain_ram_mut()
        .map_err(|_| FaceError::Inference)?;
    let planes = storage
        .as_slice_mut::<f32>()
        .map_err(|_| FaceError::Inference)?;
    if planes.len() != tensor_size {
        return Err(FaceError::Inference);
    }
    let source = frame.as_raw();
    for y in 0..height {
        let source_row = y * width * 3;
        let target_row = y * padded_width;
        for x in 0..width {
            let source_index = source_row + x * 3;
            let target_index = target_row + x;
            planes[target_index] = f32::from(source[source_index + 2]);
            planes[plane_size + target_index] = f32::from(source[source_index + 1]);
            planes[2 * plane_size + target_index] = f32::from(source[source_index]);
        }
    }
    Ok(input)
}

pub(super) fn decode_detections(
    outputs: &[TValue],
    width: u32,
    height: u32,
) -> Result<Detection, FaceError> {
    if outputs.len() != DETECTOR_OUTPUT_COUNT {
        return Err(FaceError::Inference);
    }
    if width == 0 || height == 0 {
        return Err(FaceError::NotSingleFace);
    }

    let padded_width = pad_to_32(width as usize)?;
    let padded_height = pad_to_32(height as usize)?;
    let mut candidates = Vec::new();
    let mut order = 0usize;

    for (level, stride) in DETECTOR_STRIDES.iter().copied().enumerate() {
        let cols = padded_width / stride;
        let rows = padded_height / stride;
        let cells = rows.checked_mul(cols).ok_or(FaceError::Inference)?;
        let cls = validated_output(&outputs[level], cells, 1)?;
        let obj = validated_output(&outputs[level + DETECTOR_STRIDES.len()], cells, 1)?;
        let bbox = validated_output(&outputs[level + 2 * DETECTOR_STRIDES.len()], cells, 4)?;
        let kps = validated_output(&outputs[level + 3 * DETECTOR_STRIDES.len()], cells, 10)?;
        let stride_f32 = stride as f32;

        for cell in 0..cells {
            let cls_score = cls[cell].clamp(0.0, 1.0);
            let obj_score = obj[cell].clamp(0.0, 1.0);
            let score = (cls_score * obj_score).sqrt();
            if score < DETECTOR_SCORE_THRESHOLD {
                order += 1;
                continue;
            }

            let row = cell / cols;
            let col = cell % cols;
            let bbox_offset = cell * 4;
            let center_x = (col as f32 + bbox[bbox_offset]) * stride_f32;
            let center_y = (row as f32 + bbox[bbox_offset + 1]) * stride_f32;
            let box_width = bbox[bbox_offset + 2].exp() * stride_f32;
            let box_height = bbox[bbox_offset + 3].exp() * stride_f32;
            let left = center_x - box_width / 2.0;
            let top = center_y - box_height / 2.0;
            let detection_bbox = [left, top, box_width, box_height];
            if detection_bbox.iter().any(|value| !value.is_finite())
                || box_width <= 0.0
                || box_height <= 0.0
            {
                return Err(FaceError::Inference);
            }

            let mut landmarks = [[0.0; 2]; 5];
            let landmark_offset = cell * 10;
            for (point, landmark) in landmarks.iter_mut().enumerate() {
                landmark[0] = (kps[landmark_offset + point * 2] + col as f32) * stride_f32;
                landmark[1] = (kps[landmark_offset + point * 2 + 1] + row as f32) * stride_f32;
            }
            if landmarks.iter().flatten().any(|value| !value.is_finite()) {
                return Err(FaceError::Inference);
            }

            let rect = IntegerRect {
                x: trunc_i32(left)?,
                y: trunc_i32(top)?,
                width: trunc_i32(box_width)?,
                height: trunc_i32(box_height)?,
            };
            candidates.push(Candidate {
                detection: Detection {
                    bbox: detection_bbox,
                    landmarks,
                },
                score,
                order,
                rect,
            });
            order += 1;
        }
    }

    if candidates.is_empty() {
        return Err(FaceError::NotSingleFace);
    }
    candidates.sort_unstable_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.order.cmp(&right.order))
    });
    candidates.truncate(DETECTOR_TOP_K);

    let mut kept: Vec<usize> = Vec::with_capacity(candidates.len());
    for index in 0..candidates.len() {
        let suppressed = kept.iter().any(|&kept_index| {
            intersection_over_union(candidates[index].rect, candidates[kept_index].rect)
                > DETECTOR_NMS_THRESHOLD
        });
        if !suppressed {
            kept.push(index);
        }
    }

    if kept.len() != 1 {
        return Err(FaceError::NotSingleFace);
    }
    Ok(candidates[kept[0]].detection)
}

pub(super) fn align_face(frame: &RgbImage, face: &Detection) -> Result<Tensor, FaceError> {
    if frame.width() == 0 || frame.height() == 0 {
        return Err(FaceError::Inference);
    }
    if face
        .landmarks
        .iter()
        .flatten()
        .any(|value| !value.is_finite())
    {
        return Err(FaceError::Inference);
    }
    let (transform_a, transform_b, translate_x, translate_y, determinant) =
        alignment_transform(&face.landmarks)?;

    // SAFETY: every element of all three planes is written below before use.
    let mut aligned = unsafe { Tensor::uninitialized::<f32>(&[1, 3, FACE_SIZE, FACE_SIZE]) }
        .map_err(|_| FaceError::Inference)?;
    let plane_size = FACE_SIZE * FACE_SIZE;
    let mut storage = aligned
        .try_as_plain_ram_mut()
        .map_err(|_| FaceError::Inference)?;
    let destination = storage
        .as_slice_mut::<f32>()
        .map_err(|_| FaceError::Inference)?;
    for y in 0..FACE_SIZE {
        for x in 0..FACE_SIZE {
            let dx = x as f64 - translate_x;
            let dy = y as f64 - translate_y;
            let source_x = (transform_a * dx + transform_b * dy) / determinant;
            let source_y = (-transform_b * dx + transform_a * dy) / determinant;
            if !source_x.is_finite() || !source_y.is_finite() {
                return Err(FaceError::Inference);
            }
            let rgb = bilinear_rgb(frame, source_x, source_y);
            let index = y * FACE_SIZE + x;
            destination[index] = rgb[0] as f32;
            destination[plane_size + index] = rgb[1] as f32;
            destination[2 * plane_size + index] = rgb[2] as f32;
        }
    }
    Ok(aligned)
}

pub(super) fn liveness_input(
    frame: &RgbImage,
    bbox: [f32; 4],
    requested_scale: f64,
) -> Result<Tensor, FaceError> {
    let source_width = f64::from(frame.width());
    let source_height = f64::from(frame.height());
    if frame.width() == 0
        || frame.height() == 0
        || bbox.iter().any(|value| !value.is_finite())
        || !requested_scale.is_finite()
    {
        return Err(FaceError::Inference);
    }

    let [x, y, box_width, box_height] = bbox.map(f64::from);
    if box_width <= 0.0 || box_height <= 0.0 {
        return Err(FaceError::Inference);
    }
    let scale = ((source_height - 1.0) / box_height)
        .min((source_width - 1.0) / box_width)
        .min(requested_scale);
    if !scale.is_finite() || scale <= 0.0 {
        return Err(FaceError::Inference);
    }

    let new_width = box_width * scale;
    let new_height = box_height * scale;
    let center_x = box_width / 2.0 + x;
    let center_y = box_height / 2.0 + y;
    let mut left = center_x - new_width / 2.0;
    let mut top = center_y - new_height / 2.0;
    let mut right = center_x + new_width / 2.0;
    let mut bottom = center_y + new_height / 2.0;

    // Translate the opposite edge when the expanded crop crosses the near edge,
    // then clip the far edge, matching the pinned liveness crop's inclusive bounds.
    if left < 0.0 {
        right -= left;
        left = 0.0;
    }
    if top < 0.0 {
        bottom -= top;
        top = 0.0;
    }
    if right > source_width - 1.0 {
        left -= right - source_width + 1.0;
        right = source_width - 1.0;
    }
    if bottom > source_height - 1.0 {
        top -= bottom - source_height + 1.0;
        bottom = source_height - 1.0;
    }
    if [left, top, right, bottom]
        .iter()
        .any(|value| !value.is_finite())
    {
        return Err(FaceError::Inference);
    }

    // Truncate before validating, as in the original Python int() crop. Boundary
    // translation can leave a negative subpixel rounding residue that truncates
    // to zero; rejecting it would turn valid faces into inference failures.
    let [x0, y0, x1, y1] = [left, top, right, bottom].map(|value| value as i64);
    if x0 < 0
        || y0 < 0
        || x1 < x0
        || y1 < y0
        || x1 >= i64::from(frame.width())
        || y1 >= i64::from(frame.height())
    {
        return Err(FaceError::Inference);
    }
    let [x0, y0, x1, y1] = [x0, y0, x1, y1].map(|value| value as usize);
    let crop_width = x1
        .checked_sub(x0)
        .and_then(|value| value.checked_add(1))
        .ok_or(FaceError::Inference)?;
    let crop_height = y1
        .checked_sub(y0)
        .and_then(|value| value.checked_add(1))
        .ok_or(FaceError::Inference)?;
    let frame_width = frame.width() as usize;
    let frame_height = frame.height() as usize;
    let expected_size = frame_width
        .checked_mul(frame_height)
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or(FaceError::Inference)?;
    if x1 >= frame_width || y1 >= frame_height || frame.as_raw().len() != expected_size {
        return Err(FaceError::Inference);
    }

    // SAFETY: the loops initialize every element in all three output planes.
    let mut input = unsafe { Tensor::uninitialized::<f32>(&[1, 3, LIVE_SIZE, LIVE_SIZE]) }
        .map_err(|_| FaceError::Inference)?;
    let plane_size = LIVE_SIZE * LIVE_SIZE;
    let mut storage = input
        .try_as_plain_ram_mut()
        .map_err(|_| FaceError::Inference)?;
    let destination = storage
        .as_slice_mut::<f32>()
        .map_err(|_| FaceError::Inference)?;
    let source = frame.as_raw();
    for dy in 0..LIVE_SIZE {
        let (y_low, y_high, y_weight) = resize_coordinate(dy, crop_height);
        let source_y0 = y0 + y_low;
        let source_y1 = y0 + y_high;
        for dx in 0..LIVE_SIZE {
            let (x_low, x_high, x_weight) = resize_coordinate(dx, crop_width);
            let source_x0 = x0 + x_low;
            let source_x1 = x0 + x_high;
            let index = dy * LIVE_SIZE + dx;
            for channel in 0..3 {
                // Input is RGB; MiniFASNet expects raw BGR NCHW values.
                let source_channel = 2 - channel;
                let top_left =
                    source_pixel(source, frame_width, source_x0, source_y0, source_channel);
                let top_right =
                    source_pixel(source, frame_width, source_x1, source_y0, source_channel);
                let bottom_left =
                    source_pixel(source, frame_width, source_x0, source_y1, source_channel);
                let bottom_right =
                    source_pixel(source, frame_width, source_x1, source_y1, source_channel);
                let top_value = top_left * (1.0 - x_weight) + top_right * x_weight;
                let bottom_value = bottom_left * (1.0 - x_weight) + bottom_right * x_weight;
                let value = top_value * (1.0 - y_weight) + bottom_value * y_weight;
                let rounded = value.round().clamp(0.0, 255.0) as u8;
                destination[channel * plane_size + index] = f32::from(rounded);
            }
        }
    }
    Ok(input)
}

fn pad_to_32(value: usize) -> Result<usize, FaceError> {
    value
        .checked_add(31)
        .map(|rounded| rounded / 32 * 32)
        .filter(|padded| *padded != 0)
        .ok_or(FaceError::Inference)
}

fn validated_output(value: &TValue, cells: usize, channels: usize) -> Result<&[f32], FaceError> {
    let expected_shape = [1, cells, channels];
    if value.shape() != expected_shape {
        return Err(FaceError::Inference);
    }
    let values = value
        .try_as_plain_ram()
        .and_then(|view| view.as_slice::<f32>())
        .map_err(|_| FaceError::Inference)?;
    if values.len() != cells.checked_mul(channels).ok_or(FaceError::Inference)?
        || values.iter().any(|value| !value.is_finite())
    {
        return Err(FaceError::Inference);
    }
    Ok(values)
}

#[derive(Clone, Copy)]
struct IntegerRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

struct Candidate {
    detection: Detection,
    score: f32,
    order: usize,
    rect: IntegerRect,
}

fn trunc_i32(value: f32) -> Result<i32, FaceError> {
    let truncated = f64::from(value).trunc();
    if truncated < f64::from(i32::MIN) || truncated > f64::from(i32::MAX) {
        return Err(FaceError::Inference);
    }
    Ok(truncated as i32)
}

fn intersection_over_union(left: IntegerRect, right: IntegerRect) -> f64 {
    let left_x0 = i64::from(left.x);
    let left_y0 = i64::from(left.y);
    let right_x0 = i64::from(right.x);
    let right_y0 = i64::from(right.y);
    let left_x1 = left_x0 + i64::from(left.width);
    let left_y1 = left_y0 + i64::from(left.height);
    let right_x1 = right_x0 + i64::from(right.width);
    let right_y1 = right_y0 + i64::from(right.height);
    let intersection_width = (left_x1.min(right_x1) - left_x0.max(right_x0)).max(0);
    let intersection_height = (left_y1.min(right_y1) - left_y0.max(right_y0)).max(0);
    let intersection = (intersection_width * intersection_height) as f64;
    let left_area = (i64::from(left.width).max(0) * i64::from(left.height).max(0)) as f64;
    let right_area = (i64::from(right.width).max(0) * i64::from(right.height).max(0)) as f64;
    let union = left_area + right_area - intersection;
    if union <= 0.0 {
        0.0
    } else {
        intersection / union
    }
}

fn alignment_transform(landmarks: &[[f32; 2]; 5]) -> Result<(f64, f64, f64, f64, f64), FaceError> {
    const DESTINATION: [[f64; 2]; 5] = [
        [38.2946, 51.6963],
        [73.5318, 51.5014],
        [56.0252, 71.7366],
        [41.5493, 92.3655],
        [70.7299, 92.2041],
    ];
    const DESTINATION_MEAN: [f64; 2] = [56.0262, 71.9008];

    let mut source_mean = [0.0; 2];
    for landmark in landmarks {
        source_mean[0] += f64::from(landmark[0]) / 5.0;
        source_mean[1] += f64::from(landmark[1]) / 5.0;
    }

    let mut variance = 0.0;
    let mut dot = 0.0;
    let mut cross = 0.0;
    for index in 0..landmarks.len() {
        let source_x = f64::from(landmarks[index][0]) - source_mean[0];
        let source_y = f64::from(landmarks[index][1]) - source_mean[1];
        let destination_x = DESTINATION[index][0] - DESTINATION_MEAN[0];
        let destination_y = DESTINATION[index][1] - DESTINATION_MEAN[1];
        variance += source_x * source_x + source_y * source_y;
        dot += source_x * destination_x + source_y * destination_y;
        cross += source_x * destination_y - source_y * destination_x;
    }
    let a = dot / variance;
    let b = cross / variance;
    let determinant = a * a + b * b;
    let translate_x = DESTINATION_MEAN[0] - a * source_mean[0] + b * source_mean[1];
    let translate_y = DESTINATION_MEAN[1] - b * source_mean[0] - a * source_mean[1];
    if !variance.is_finite()
        || variance <= f64::EPSILON
        || !determinant.is_finite()
        || determinant <= f64::EPSILON
        || !translate_x.is_finite()
        || !translate_y.is_finite()
    {
        return Err(FaceError::Inference);
    }
    Ok((a, b, translate_x, translate_y, determinant))
}

fn bilinear_rgb(frame: &RgbImage, x: f64, y: f64) -> [u8; 3] {
    let width = frame.width() as i64;
    let height = frame.height() as i64;
    if x <= -1.0 || y <= -1.0 || x >= width as f64 || y >= height as f64 {
        return [0; 3];
    }

    let x0 = x.floor() as i64;
    let y0 = y.floor() as i64;
    let x1 = x0 + 1;
    let y1 = y0 + 1;
    let x_weight = x - x0 as f64;
    let y_weight = y - y0 as f64;
    let mut pixel = [0; 3];
    for channel in 0..3 {
        let top_left = pixel_or_black(frame, x0, y0, channel);
        let top_right = pixel_or_black(frame, x1, y0, channel);
        let bottom_left = pixel_or_black(frame, x0, y1, channel);
        let bottom_right = pixel_or_black(frame, x1, y1, channel);
        let top = f64::from(top_left) * (1.0 - x_weight) + f64::from(top_right) * x_weight;
        let bottom = f64::from(bottom_left) * (1.0 - x_weight) + f64::from(bottom_right) * x_weight;
        pixel[channel] = (top * (1.0 - y_weight) + bottom * y_weight)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    pixel
}

fn pixel_or_black(frame: &RgbImage, x: i64, y: i64, channel: usize) -> u8 {
    if x < 0 || y < 0 || x >= i64::from(frame.width()) || y >= i64::from(frame.height()) {
        0
    } else {
        frame.get_pixel(x as u32, y as u32).0[channel]
    }
}

fn resize_coordinate(destination: usize, source_length: usize) -> (usize, usize, f64) {
    let coordinate = (destination as f64 + 0.5) * (source_length as f64 / LIVE_SIZE as f64) - 0.5;
    if coordinate <= 0.0 {
        (0, 0, 0.0)
    } else if coordinate >= source_length as f64 - 1.0 {
        (source_length - 1, source_length - 1, 0.0)
    } else {
        let low = coordinate.floor() as usize;
        (low, low + 1, coordinate - low as f64)
    }
}

fn source_pixel(source: &[u8], frame_width: usize, x: usize, y: usize, channel: usize) -> f64 {
    f64::from(source[(y * frame_width + x) * 3 + channel])
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    #[test]
    fn nms_suppresses_duplicate_boxes_but_rejects_two_real_faces() {
        let mut duplicate = empty_detector_outputs();
        set_detection(&mut duplicate, 0, 0, 0.95, 0.0, 0.0);
        set_detection(&mut duplicate, 0, 1, 0.94, -1.0, 0.0);
        let duplicate = into_outputs(duplicate);
        let face = decode_detections(&duplicate, 32, 32).unwrap();
        assert_eq!(face.bbox, [-4.0, -4.0, 8.0, 8.0]);

        let mut two_faces = empty_detector_outputs();
        set_detection(&mut two_faces, 0, 0, 0.95, 0.0, 0.0);
        set_detection(&mut two_faces, 0, 1, 0.94, -1.0, 0.0);
        set_detection(&mut two_faces, 0, 10, 0.93, 0.0, 0.0);
        let two_faces = into_outputs(two_faces);
        assert_eq!(
            decode_detections(&two_faces, 32, 32),
            Err(FaceError::NotSingleFace)
        );
    }

    #[test]
    fn liveness_crop_translates_at_edges_and_uses_bgr_bytes() {
        let frame = RgbImage::from_fn(8, 8, |x, y| {
            Rgb([10 + x as u8, 20 + y as u8, 30 + x as u8 + y as u8])
        });
        let input = liveness_input(&frame, [0.5, 0.5, 2.0, 2.0], 4.0).unwrap();
        assert_eq!(input.shape(), &[1, 3, 80, 80]);
        let values = input.try_as_plain_ram().unwrap().as_slice::<f32>().unwrap();
        let plane_size = 80 * 80;
        assert_eq!(&values[..1], &[30.0]);
        assert_eq!(values[plane_size], 20.0);
        assert_eq!(values[2 * plane_size], 10.0);
        let last_pixel = 79 * 80 + 79;
        assert_eq!(values[last_pixel], 44.0);
        assert_eq!(values[plane_size + last_pixel], 27.0);
        assert_eq!(values[2 * plane_size + last_pixel], 17.0);
    }

    #[test]
    fn liveness_crop_truncates_subpixel_boundary_roundoff() {
        let frame = RgbImage::from_fn(480, 640, |x, _| Rgb([(x / 3) as u8, 50, 0]));
        let input = liveness_input(&frame, [170.0, 100.0, 200.001, 150.0], 2.7).unwrap();
        let values = input.try_as_plain_ram().unwrap().as_slice::<f32>().unwrap();
        assert_eq!(values[2 * 80 * 80], 1.0);
        assert_eq!(values[3 * 80 * 80 - 1], 159.0);
    }

    #[test]
    fn alignment_rejects_singular_landmarks() {
        let frame = RgbImage::from_pixel(2, 2, Rgb([1, 2, 3]));
        let face = Detection {
            bbox: [0.0, 0.0, 1.0, 1.0],
            landmarks: [[1.0, 1.0]; 5],
        };
        assert!(matches!(
            align_face(&frame, &face),
            Err(FaceError::Inference)
        ));
    }

    #[test]
    fn alignment_recovers_rotation_scale_and_translation() {
        let destination: [[f32; 2]; 5] = [
            [38.2946, 51.6963],
            [73.5318, 51.5014],
            [56.0252, 71.7366],
            [41.5493, 92.3655],
            [70.7299, 92.2041],
        ];
        let source = destination.map(|[x, y]| [-2.0 * y + 100.0, 2.0 * x + 20.0]);
        let (a, b, tx, ty, _) = alignment_transform(&source).unwrap();
        for ([x, y], expected) in source.into_iter().zip(destination) {
            let actual_x = a * f64::from(x) - b * f64::from(y) + tx;
            let actual_y = b * f64::from(x) + a * f64::from(y) + ty;
            assert!((actual_x - f64::from(expected[0])).abs() < 0.001);
            assert!((actual_y - f64::from(expected[1])).abs() < 0.001);
        }
    }

    fn empty_detector_outputs() -> Vec<Tensor> {
        let cells = [16, 4, 1];
        let mut outputs = Vec::with_capacity(DETECTOR_OUTPUT_COUNT);
        for output_index in 0..DETECTOR_OUTPUT_COUNT {
            let level = output_index % 3;
            let channels = if output_index < 6 {
                1
            } else if output_index < 9 {
                4
            } else {
                10
            };
            outputs.push(Tensor::zero::<f32>(&[1, cells[level], channels]).unwrap());
        }
        outputs
    }

    fn set_detection(
        outputs: &mut [Tensor],
        level: usize,
        cell: usize,
        score: f32,
        x_offset: f32,
        y_offset: f32,
    ) {
        outputs[level]
            .try_as_plain_ram_mut()
            .unwrap()
            .as_slice_mut::<f32>()
            .unwrap()[cell] = score;
        outputs[level + 3]
            .try_as_plain_ram_mut()
            .unwrap()
            .as_slice_mut::<f32>()
            .unwrap()[cell] = score;
        let mut storage = outputs[level + 6].try_as_plain_ram_mut().unwrap();
        let bbox = storage.as_slice_mut::<f32>().unwrap();
        bbox[cell * 4] = x_offset;
        bbox[cell * 4 + 1] = y_offset;
    }

    fn into_outputs(outputs: Vec<Tensor>) -> Vec<TValue> {
        outputs.into_iter().map(Into::into).collect()
    }
}
