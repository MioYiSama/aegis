use image::{GrayImage, Luma, Rgb, RgbImage};
use imageproc::{
    contours::{BorderType, Contour, find_contours_with_threshold},
    geometric_transformations::Projection,
    geometry::{approximate_polygon_dp, arc_length, contour_area},
    point::Point,
};
use qrcode::{Color as QrColor, EcLevel, QrCode, Version};
use std::collections::HashSet;
use thiserror::Error;

const SYMBOL_MODULES: usize = 29;
const QUIET_MODULES: usize = 4;
const CARRIER_MODULES: usize = SYMBOL_MODULES + QUIET_MODULES * 2;
const MODULE_PIXELS: usize = 16;
const CHIP_PIXELS: usize = 4;
const MARGIN_PIXELS: usize = 16;
const BORDER_PIXELS: usize = 8;
const GAP_PIXELS: usize = 8;
const PAYLOAD_OFFSET: usize = MARGIN_PIXELS + BORDER_PIXELS + GAP_PIXELS;
const FRAME_PADDING: usize = 2 * PAYLOAD_OFFSET;
const OUTPUT_MODULE_PIXELS: usize = 8;
const OUTPUT_QUIET_MODULES: usize = 4;
const ALIGNMENT_GATE_HITS: u16 = 106;
const MAX_QUIET_SCAN: usize = 8;
const PHASE_GRID: usize = 9;
const MAX_PHASE_ATTEMPTS: usize = 8;
const COLOR_LIGHT: [u8; 3] = [210, 74, 120];
const COLOR_DARK: [u8; 3] = [30, 166, 120];
const FINDER: [[bool; 7]; 7] = [
    [true, true, true, true, true, true, true],
    [true, false, false, false, false, false, true],
    [true, false, true, true, true, false, true],
    [true, false, true, true, true, false, true],
    [true, false, true, true, true, false, true],
    [true, false, false, false, false, false, true],
    [true, true, true, true, true, true, true],
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum QrError {
    #[error("invalid QR payload")]
    InvalidPayload,
    #[error("QR carrier is unreadable")]
    Unreadable,
}

/// Encode an ASCII payload as the fixed version-3 chroma-carrier QR image.
pub fn encode_chroma(payload: &str) -> Result<RgbImage, QrError> {
    if payload.is_empty() || !payload.is_ascii() || payload.len() > 42 {
        return Err(QrError::InvalidPayload);
    }

    let code = QrCode::with_version(payload.as_bytes(), Version::Normal(3), EcLevel::M)
        .map_err(|_| QrError::InvalidPayload)?;
    if code.width() != SYMBOL_MODULES {
        return Err(QrError::InvalidPayload);
    }

    let side = CARRIER_MODULES * MODULE_PIXELS + FRAME_PADDING;
    let mut carrier = RgbImage::from_pixel(side as u32, side as u32, Rgb([255, 255, 255]));

    let ring_start = MARGIN_PIXELS;
    let ring_end = side - MARGIN_PIXELS;
    let inner_start = ring_start + BORDER_PIXELS;
    let inner_end = ring_end - BORDER_PIXELS;
    for y in ring_start..ring_end {
        for x in ring_start..ring_end {
            if x < inner_start || x >= inner_end || y < inner_start || y >= inner_end {
                carrier.put_pixel(x as u32, y as u32, Rgb([0, 0, 0]));
            }
        }
    }

    for module_row in 0..CARRIER_MODULES {
        for module_col in 0..CARRIER_MODULES {
            let dark_module = if (QUIET_MODULES..QUIET_MODULES + SYMBOL_MODULES)
                .contains(&module_row)
                && (QUIET_MODULES..QUIET_MODULES + SYMBOL_MODULES).contains(&module_col)
            {
                code[(module_col - QUIET_MODULES, module_row - QUIET_MODULES)] == QrColor::Dark
            } else {
                false
            };
            let origin_y = PAYLOAD_OFFSET + module_row * MODULE_PIXELS;
            let origin_x = PAYLOAD_OFFSET + module_col * MODULE_PIXELS;
            for pixel_row in 0..MODULE_PIXELS {
                for pixel_col in 0..MODULE_PIXELS {
                    let checker = ((pixel_row / CHIP_PIXELS + pixel_col / CHIP_PIXELS) & 1) != 0;
                    let color = if checker ^ dark_module {
                        COLOR_DARK
                    } else {
                        COLOR_LIGHT
                    };
                    carrier.put_pixel(
                        (origin_x + pixel_col) as u32,
                        (origin_y + pixel_row) as u32,
                        Rgb(color),
                    );
                }
            }
        }
    }

    Ok(carrier)
}

/// Recover QR text from a photograph of the chroma carrier.
pub fn decode_chroma(photo: &RgbImage) -> Result<String, QrError> {
    decode_chroma_region(photo).map(|(payload, _)| payload)
}

/// Recover QR text and the carrier's bounding rectangle in original pixels.
pub fn decode_chroma_region(photo: &RgbImage) -> Result<(String, [u32; 4]), QrError> {
    if photo.width() < 3 || photo.height() < 3 {
        return Err(QrError::Unreadable);
    }
    if photo.width() > i32::MAX as u32 || photo.height() > i32::MAX as u32 {
        return Err(QrError::Unreadable);
    }
    let gray = grayscale_rec601(photo);
    let quads = detect_frame_quads(&gray);
    decode_frame_candidates(photo, &quads).ok_or(QrError::Unreadable)
}

#[derive(Clone, Copy, Debug)]
struct PointF {
    x: f64,
    y: f64,
}

type Quad = [PointF; 4];

#[derive(Debug)]
struct FrameQuad {
    quad: Quad,
    inset: f32,
    area: f64,
}

fn grayscale_rec601(image: &RgbImage) -> GrayImage {
    let mut gray = GrayImage::new(image.width(), image.height());
    for (source, target) in image.pixels().zip(gray.pixels_mut()) {
        target.0[0] = luma_rec601(source);
    }
    gray
}

fn luma_rec601(pixel: &Rgb<u8>) -> u8 {
    let [red, green, blue] = pixel.0;
    let value = 0.299 * f64::from(red) + 0.587 * f64::from(green) + 0.114 * f64::from(blue);
    value.round().clamp(0.0, 255.0) as u8
}

fn percentile(histogram: &[usize; 256], count: usize, percent: f64) -> f64 {
    let position = percent * (count - 1) as f64 / 100.0;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    let fraction = position - lower as f64;
    f64::from(order_statistic(histogram, lower)) * (1.0 - fraction)
        + f64::from(order_statistic(histogram, upper)) * fraction
}

fn order_statistic(histogram: &[usize; 256], rank: usize) -> u8 {
    let mut cumulative = 0;
    for (value, frequency) in histogram.iter().enumerate() {
        cumulative += frequency;
        if cumulative > rank {
            return value as u8;
        }
    }
    255
}

fn sampled_quantiles(gray: &GrayImage) -> (f64, f64) {
    let step = (gray.width().min(gray.height()) as usize / 512).max(1);
    let mut histogram = [0_usize; 256];
    let mut count = 0;
    for y in (0..gray.height()).step_by(step) {
        for x in (0..gray.width()).step_by(step) {
            histogram[gray.get_pixel(x, y).0[0] as usize] += 1;
            count += 1;
        }
    }
    (
        percentile(&histogram, count, 1.0),
        // The white frame occupies much less of a camera view than the scene.
        // A scene median (or even p90) can be darker than the carrier's ring.
        percentile(&histogram, count, 99.0),
    )
}

fn detect_frame_quads(gray: &GrayImage) -> Vec<FrameQuad> {
    let (dark, bright) = sampled_quantiles(gray);
    let width = gray.width() as usize;
    let height = gray.height() as usize;
    let min_area = 0.002 * (width * height) as f64;
    let mut ring_like = Vec::new();
    let mut other = Vec::new();
    let mut seen: HashSet<[i32; 8]> = HashSet::new();

    for factor in [0.35, 0.6, 0.8, 0.95] {
        let threshold = dark + factor * (bright - dark);
        if threshold - dark < 1.0 {
            continue;
        }

        let mask = GrayImage::from_fn(gray.width(), gray.height(), |x, y| {
            if f64::from(gray.get_pixel(x, y).0[0]) <= threshold {
                Luma([255])
            } else {
                Luma([0])
            }
        });
        let contours = find_contours_with_threshold::<i32>(&mask, 0);

        for (index, contour) in contours.iter().enumerate() {
            if contour.border_type != BorderType::Outer {
                continue;
            }
            let area = contour_area(&contour.points);
            if area < min_area {
                continue;
            }
            let Some(outer) = contour_quad(contour) else {
                continue;
            };

            let mut quad = outer;
            let mut inset = MARGIN_PIXELS as f32;
            let mut hole_fraction = 0.0;
            if let Some(child) = contours
                .iter()
                .find(|child| child.parent == Some(index) && child.border_type == BorderType::Hole)
            {
                if area > 0.0 {
                    hole_fraction = contour_area(&child.points) / area;
                }
                if let Some(inner) = contour_quad(child) {
                    if quads_nested(&outer, &inner) {
                        quad = std::array::from_fn(|corner| PointF {
                            x: (outer[corner].x + inner[corner].x) * 0.5,
                            y: (outer[corner].y + inner[corner].y) * 0.5,
                        });
                        inset = (MARGIN_PIXELS + BORDER_PIXELS / 2) as f32;
                    }
                }
            }

            let key = std::array::from_fn(|index| {
                let point = quad[index / 2];
                if index % 2 == 0 {
                    point.x.round() as i32
                } else {
                    point.y.round() as i32
                }
            });
            if !seen.insert(key) {
                continue;
            }
            let candidate = FrameQuad { quad, inset, area };
            if hole_fraction >= 0.6 {
                ring_like.push(candidate);
            } else {
                other.push(candidate);
            }
        }
    }

    ring_like.sort_by(|left, right| right.area.total_cmp(&left.area));
    other.sort_by(|left, right| right.area.total_cmp(&left.area));
    ring_like.extend(other);
    ring_like.truncate(4);
    ring_like
}

fn contour_quad(contour: &Contour<i32>) -> Option<Quad> {
    let points = &contour.points;
    if points.len() < 4 {
        return None;
    }
    let perimeter = arc_length(points, true);
    if !perimeter.is_finite() || perimeter <= 0.0 {
        return None;
    }

    for factor in [0.02, 0.035, 0.05] {
        let approximated = approximate_closed_curve(points, factor * perimeter)?;
        if approximated.len() != 4 || !is_convex_quad(&approximated) {
            continue;
        }
        let quad = order_quad(&approximated);
        let mut minimum = f64::INFINITY;
        let mut maximum = 0.0_f64;
        for edge in 0..4 {
            let first = quad[edge];
            let second = quad[(edge + 1) % 4];
            let length = (first.x - second.x).hypot(first.y - second.y);
            minimum = minimum.min(length);
            maximum = maximum.max(length);
        }
        if minimum > 0.0 && maximum / minimum <= 2.0 {
            return Some(fit_quad_edges(points, &quad).unwrap_or(quad));
        }
    }
    None
}

#[derive(Clone, Copy, Default)]
struct LineMoments {
    count: usize,
    x: f64,
    y: f64,
    xx: f64,
    xy: f64,
    yy: f64,
}

fn fit_quad_edges(points: &[Point<i32>], quad: &Quad) -> Option<Quad> {
    // Polygon simplification selects integer contour vertices. A subpixel
    // error there moves the high-frequency chip grid across the entire image.
    // Fit each straight edge from its interior, then intersect adjacent lines.
    let mut moments = [LineMoments::default(); 4];
    for point in points {
        let x = f64::from(point.x);
        let y = f64::from(point.y);
        let mut nearest = None;
        let mut minimum_distance = f64::INFINITY;
        for edge in 0..4 {
            let start = quad[edge];
            let end = quad[(edge + 1) % 4];
            let dx = end.x - start.x;
            let dy = end.y - start.y;
            let length_squared = dx * dx + dy * dy;
            let position = ((x - start.x) * dx + (y - start.y) * dy) / length_squared;
            if !(0.1..=0.9).contains(&position) {
                continue;
            }
            let cross = (x - start.x) * dy - (y - start.y) * dx;
            let distance = cross * cross / length_squared;
            if distance <= length_squared * 0.0009 && distance < minimum_distance {
                minimum_distance = distance;
                nearest = Some(edge);
            }
        }
        if let Some(edge) = nearest {
            let sums = &mut moments[edge];
            sums.count += 1;
            sums.x += x;
            sums.y += y;
            sums.xx += x * x;
            sums.xy += x * y;
            sums.yy += y * y;
        }
    }
    let mut lines = [(0.0, 0.0, 0.0); 4];
    for (edge, sums) in moments.iter().enumerate() {
        if sums.count < 8 {
            return None;
        }
        let count = sums.count as f64;
        let x = sums.x / count;
        let y = sums.y / count;
        let xx = sums.xx / count - x * x;
        let xy = sums.xy / count - x * y;
        let yy = sums.yy / count - y * y;
        let angle = 0.5 * (2.0 * xy).atan2(xx - yy);
        let (sin, cos) = angle.sin_cos();
        let (a, b) = (-sin, cos);
        lines[edge] = (a, b, a * x + b * y);
    }
    let mut refined = *quad;
    for corner in 0..4 {
        let (a, b, c) = lines[(corner + 3) % 4];
        let (d, e, f) = lines[corner];
        let denominator = a * e - d * b;
        if denominator.abs() < 1e-6 {
            return None;
        }
        let point = PointF {
            x: (c * e - f * b) / denominator,
            y: (a * f - d * c) / denominator,
        };
        let old = quad[corner];
        let previous = quad[(corner + 3) % 4];
        let next = quad[(corner + 1) % 4];
        let edge_squared = ((old.x - previous.x).powi(2) + (old.y - previous.y).powi(2))
            .min((old.x - next.x).powi(2) + (old.y - next.y).powi(2));
        if !point.x.is_finite()
            || !point.y.is_finite()
            || (point.x - old.x).powi(2) + (point.y - old.y).powi(2) > edge_squared * 0.0025
        {
            return None;
        }
        refined[corner] = point;
    }
    Some(refined)
}

fn approximate_closed_curve(points: &[Point<i32>], epsilon: f64) -> Option<Vec<Point<i32>>> {
    if points.len() < 4 {
        return None;
    }

    let start = farthest_point(points, 0);
    let opposite = farthest_point(points, start);
    if start == opposite {
        return None;
    }

    let first_arc = contour_arc(points, start, opposite);
    let second_arc = contour_arc(points, opposite, start);
    if first_arc.len() < 2 || second_arc.len() < 2 {
        return None;
    }
    let mut simplified = approximate_polygon_dp(&first_arc, epsilon, false);
    let second = approximate_polygon_dp(&second_arc, epsilon, false);
    if second.len() > 2 {
        simplified.extend(second[1..second.len() - 1].iter().copied());
    }
    Some(simplified)
}

fn farthest_point(points: &[Point<i32>], from: usize) -> usize {
    let origin = points[from];
    let mut farthest = from;
    let mut greatest = -1.0_f64;
    for (index, point) in points.iter().enumerate() {
        let dx = f64::from(point.x - origin.x);
        let dy = f64::from(point.y - origin.y);
        let distance_squared = dx * dx + dy * dy;
        if distance_squared > greatest {
            greatest = distance_squared;
            farthest = index;
        }
    }
    farthest
}

fn contour_arc(points: &[Point<i32>], start: usize, end: usize) -> Vec<Point<i32>> {
    let mut arc = Vec::new();
    let mut index = start;
    loop {
        arc.push(points[index]);
        if index == end {
            break;
        }
        index = (index + 1) % points.len();
    }
    arc
}

fn is_convex_quad(points: &[Point<i32>]) -> bool {
    let mut sign = 0_i64;
    for index in 0..4 {
        let a = points[index];
        let b = points[(index + 1) % 4];
        let c = points[(index + 2) % 4];
        let ab_x = i64::from(b.x) - i64::from(a.x);
        let ab_y = i64::from(b.y) - i64::from(a.y);
        let bc_x = i64::from(c.x) - i64::from(b.x);
        let bc_y = i64::from(c.y) - i64::from(b.y);
        let cross = ab_x * bc_y - ab_y * bc_x;
        if cross == 0 {
            return false;
        }
        if sign == 0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    sign != 0
}

fn order_quad(points: &[Point<i32>]) -> Quad {
    let mut top_left = 0;
    let mut top_right = 0;
    let mut bottom_right = 0;
    let mut bottom_left = 0;
    for index in 1..4 {
        let sum = points[index].x + points[index].y;
        let tl_sum = points[top_left].x + points[top_left].y;
        let br_sum = points[bottom_right].x + points[bottom_right].y;
        let difference = points[index].x - points[index].y;
        let tr_difference = points[top_right].x - points[top_right].y;
        let bl_difference = points[bottom_left].x - points[bottom_left].y;
        if sum < tl_sum {
            top_left = index;
        }
        if sum > br_sum {
            bottom_right = index;
        }
        if difference > tr_difference {
            top_right = index;
        }
        if difference < bl_difference {
            bottom_left = index;
        }
    }
    [top_left, top_right, bottom_right, bottom_left].map(|index| PointF {
        x: f64::from(points[index].x),
        y: f64::from(points[index].y),
    })
}

fn polygon_area(quad: &Quad) -> f64 {
    let mut double_area = 0.0;
    for index in 0..4 {
        let current = quad[index];
        let next = quad[(index + 1) % 4];
        double_area += current.x * next.y - current.y * next.x;
    }
    double_area.abs() * 0.5
}

fn point_strictly_inside(point: PointF, polygon: &Quad) -> bool {
    let mut inside = false;
    for index in 0..4 {
        let first = polygon[index];
        let second = polygon[(index + 1) % 4];
        let dx = second.x - first.x;
        let dy = second.y - first.y;
        let cross = (point.x - first.x) * dy - (point.y - first.y) * dx;
        let dot =
            (point.x - first.x) * (point.x - second.x) + (point.y - first.y) * (point.y - second.y);
        if cross.abs() < 1e-7 && dot <= 0.0 {
            return false;
        }
        if (first.y > point.y) != (second.y > point.y)
            && point.x < (second.x - first.x) * (point.y - first.y) / (second.y - first.y) + first.x
        {
            inside = !inside;
        }
    }
    inside
}

fn quads_nested(outer: &Quad, inner: &Quad) -> bool {
    polygon_area(inner) < polygon_area(outer)
        && inner
            .iter()
            .all(|corner| point_strictly_inside(*corner, outer))
}

#[derive(Debug)]
struct Hypothesis {
    modules: usize,
    quiet: usize,
    rotation: usize,
    polarity: bool,
    correlations: Vec<f32>,
}

fn decode_frame_candidates(photo: &RgbImage, quads: &[FrameQuad]) -> Option<(String, [u32; 4])> {
    for frame_quad in quads {
        for modules in [37, 45, 29, 41, 33] {
            let Some(frame) = warp_frame(photo, &frame_quad.quad, modules, frame_quad.inset) else {
                continue;
            };
            if !frame_is_plausible(&frame) {
                continue;
            }
            let correlations = module_correlations(&frame, modules);
            let (hits, quiet, rotation, polarity) = align_grid(&correlations, modules);
            if hits < ALIGNMENT_GATE_HITS {
                continue;
            }
            let hypothesis = Hypothesis {
                modules,
                quiet,
                rotation,
                polarity,
                correlations,
            };
            // Finder agreement is only alignment evidence. A damaged candidate
            // must not prevent trying the remaining frames and module sizes.
            if let Some(text) = render_symbol(&hypothesis).and_then(decode_standard_qr) {
                return Some((text, carrier_bounds(photo, &frame_quad.quad)));
            }
            if let Some(text) = refine_symbol_phase(&frame, &hypothesis) {
                return Some((text, carrier_bounds(photo, &frame_quad.quad)));
            }
        }
    }
    None
}

fn carrier_bounds(photo: &RgbImage, quad: &Quad) -> [u32; 4] {
    let min_x = quad
        .iter()
        .map(|point| point.x)
        .fold(f64::INFINITY, f64::min);
    let min_y = quad
        .iter()
        .map(|point| point.y)
        .fold(f64::INFINITY, f64::min);
    let max_x = quad
        .iter()
        .map(|point| point.x)
        .fold(f64::NEG_INFINITY, f64::max);
    let max_y = quad
        .iter()
        .map(|point| point.y)
        .fold(f64::NEG_INFINITY, f64::max);
    // Include the full outer margin and a small contour/perspective allowance.
    // No resampling: the server receives exactly the detected camera pixels.
    let padding = (max_x - min_x).max(max_y - min_y) * 0.06 + 2.0;
    let left = (min_x - padding).floor().max(0.0) as u32;
    let top = (min_y - padding).floor().max(0.0) as u32;
    let right = (max_x + padding).ceil().min(f64::from(photo.width())) as u32;
    let bottom = (max_y + padding).ceil().min(f64::from(photo.height())) as u32;
    [left, top, right - left, bottom - top]
}

fn warp_frame(photo: &RgbImage, quad: &Quad, modules: usize, inset: f32) -> Option<RgbImage> {
    let side = modules
        .checked_mul(MODULE_PIXELS)?
        .checked_add(FRAME_PADDING)?;
    let last = side as f32 - 1.0 - inset;
    let destination = [(inset, inset), (last, inset), (last, last), (inset, last)];
    let source = quad.map(|point| (point.x as f32, point.y as f32));
    // The control-point mapping is canonical-canvas -> photo coordinates. Sampling
    // it directly lets the edge handling replicate the nearest input pixel.
    let projection = Projection::from_control_points(destination, source)?;
    let max_x = photo.width() as f32 - 1.0;
    let max_y = photo.height() as f32 - 1.0;
    let mut warped = RgbImage::new(side as u32, side as u32);

    for y in 0..side {
        for x in 0..side {
            let (sample_x, sample_y) = projection * (x as f32, y as f32);
            if !sample_x.is_finite() || !sample_y.is_finite() {
                return None;
            }
            let sample_x = sample_x.clamp(0.0, max_x);
            let sample_y = sample_y.clamp(0.0, max_y);
            let left = sample_x.floor() as u32;
            let top = sample_y.floor() as u32;
            let right = (left + 1).min(photo.width() - 1);
            let bottom = (top + 1).min(photo.height() - 1);
            let horizontal = sample_x - left as f32;
            let vertical = sample_y - top as f32;
            let top_left = photo.get_pixel(left, top).0;
            let top_right = photo.get_pixel(right, top).0;
            let bottom_left = photo.get_pixel(left, bottom).0;
            let bottom_right = photo.get_pixel(right, bottom).0;
            let color = std::array::from_fn(|channel| {
                let upper = f32::from(top_left[channel]) * (1.0 - horizontal)
                    + f32::from(top_right[channel]) * horizontal;
                let lower = f32::from(bottom_left[channel]) * (1.0 - horizontal)
                    + f32::from(bottom_right[channel]) * horizontal;
                (upper * (1.0 - vertical) + lower * vertical)
                    .round()
                    .clamp(0.0, 255.0) as u8
            });
            warped.put_pixel(x as u32, y as u32, Rgb(color));
        }
    }
    Some(warped)
}

fn frame_is_plausible(frame: &RgbImage) -> bool {
    let side = frame.width() as usize;
    let offset = PAYLOAD_OFFSET;
    let edge = side - offset;

    let mean_band = |start: usize, end: usize| {
        let mut sum = 0_u64;
        let mut count = 0_u64;
        for y in start..end {
            for x in offset..edge {
                sum += u64::from(luma_rec601(frame.get_pixel(x as u32, y as u32)));
                sum += u64::from(luma_rec601(
                    frame.get_pixel(x as u32, (side - end + y - start) as u32),
                ));
                count += 2;
            }
        }
        for x in start..end {
            for y in offset..edge {
                sum += u64::from(luma_rec601(frame.get_pixel(x as u32, y as u32)));
                sum += u64::from(luma_rec601(
                    frame.get_pixel((side - end + x - start) as u32, y as u32),
                ));
                count += 2;
            }
        }
        sum as f64 / count as f64
    };

    let ring = mean_band(MARGIN_PIXELS + 2, MARGIN_PIXELS + BORDER_PIXELS - 2);
    let gap = mean_band(MARGIN_PIXELS + BORDER_PIXELS + 2, PAYLOAD_OFFSET - 2);
    // The payload uses different colors, so its brightness is not evidence of
    // a black ring. Exposure and blur can make the thin ring brighter than it.
    ring + 15.0 < gap
}

fn module_correlations(frame: &RgbImage, modules: usize) -> Vec<f32> {
    let mut correlations = vec![0.0; modules * modules];
    for module_row in 0..modules {
        for module_col in 0..modules {
            let mut sum = 0.0_f32;
            let origin_y = PAYLOAD_OFFSET + module_row * MODULE_PIXELS;
            let origin_x = PAYLOAD_OFFSET + module_col * MODULE_PIXELS;
            for pixel_row in 0..MODULE_PIXELS {
                for pixel_col in 0..MODULE_PIXELS {
                    let pixel = frame
                        .get_pixel((origin_x + pixel_col) as u32, (origin_y + pixel_row) as u32);
                    let signal = f32::from(pixel.0[0]) - f32::from(pixel.0[1]);
                    let checker = ((pixel_row / CHIP_PIXELS + pixel_col / CHIP_PIXELS) & 1) != 0;
                    sum += if checker { -signal } else { signal };
                }
            }
            correlations[module_row * modules + module_col] =
                sum / (MODULE_PIXELS * MODULE_PIXELS) as f32;
        }
    }
    correlations
}

struct Phase {
    x: f32,
    y: f32,
    score: f32,
}

fn rank_phases(frame: &RgbImage, hypothesis: &Hypothesis) -> [Phase; PHASE_GRID * PHASE_GRID] {
    // Half-pixel steps within half a chip. Larger shifts can invert the carrier
    // phase and are not a substitute for finding the correct frame/grid.
    let centre = (PHASE_GRID / 2) as f32;
    let mut phases = std::array::from_fn(|index| {
        let x = (index % PHASE_GRID) as f32 * 0.5 - centre * 0.5;
        let y = (index / PHASE_GRID) as f32 * 0.5 - centre * 0.5;
        let mut score = 0.0;
        let symbol_size = hypothesis.modules - 2 * hypothesis.quiet;
        for &(row_offset, col_offset) in &[(0, 0), (0, symbol_size - 7), (symbol_size - 7, 0)] {
            for row in 0..7 {
                for col in 0..7 {
                    let (source_row, source_col) = module_position(
                        hypothesis.modules,
                        hypothesis.rotation,
                        hypothesis.quiet + row_offset + row,
                        hypothesis.quiet + col_offset + col,
                    );
                    let value = chip_correlation(frame, source_row, source_col, x, y);
                    score += if FINDER[row][col] ^ hypothesis.polarity {
                        -value
                    } else {
                        value
                    };
                }
            }
        }
        Phase { x, y, score }
    });
    phases.sort_unstable_by(|left, right| right.score.total_cmp(&left.score));
    phases
}

fn refine_symbol_phase(frame: &RgbImage, hypothesis: &Hypothesis) -> Option<String> {
    // Keep the original decoder as the fast path. Only a plausible, aligned
    // symbol that failed QR error correction needs this bounded phase search.
    for phase in rank_phases(frame, hypothesis)
        .iter()
        .take(MAX_PHASE_ATTEMPTS)
    {
        if phase.score <= 0.0 {
            break;
        }
        let modules = hypothesis.modules;
        let mut correlations = vec![0.0; modules * modules];
        for row in 0..modules {
            for col in 0..modules {
                correlations[row * modules + col] =
                    chip_correlation(frame, row, col, phase.x, phase.y);
            }
        }
        if finder_hits(
            &correlations,
            modules,
            hypothesis.quiet,
            hypothesis.rotation,
            hypothesis.polarity,
        ) < ALIGNMENT_GATE_HITS
        {
            continue;
        }
        let refined = Hypothesis {
            modules,
            quiet: hypothesis.quiet,
            rotation: hypothesis.rotation,
            polarity: hypothesis.polarity,
            correlations,
        };
        if let Some(text) = render_symbol(&refined).and_then(decode_standard_qr) {
            return Some(text);
        }
    }
    None
}

fn chip_correlation(frame: &RgbImage, row: usize, col: usize, dx: f32, dy: f32) -> f32 {
    let origin_x = (PAYLOAD_OFFSET + col * MODULE_PIXELS) as f32 + dx;
    let origin_y = (PAYLOAD_OFFSET + row * MODULE_PIXELS) as f32 + dy;
    let mut sum = 0.0;
    let chips = MODULE_PIXELS / CHIP_PIXELS;
    let centre = (CHIP_PIXELS - 1) as f32 * 0.5;
    for y in 0..chips {
        for x in 0..chips {
            // Chip interiors avoid mixing opposite colors at a blurred edge.
            let signal = sample_chroma(
                frame,
                origin_x + (x * CHIP_PIXELS) as f32 + centre,
                origin_y + (y * CHIP_PIXELS) as f32 + centre,
            );
            sum += if (x + y) & 1 == 0 { signal } else { -signal };
        }
    }
    sum / (chips * chips) as f32
}

fn sample_chroma(frame: &RgbImage, x: f32, y: f32) -> f32 {
    // The payload margin and bounded phase shift keep all four samples inside
    // the canonical frame, so no per-sample clamping/allocation is needed.
    let left = x.floor() as u32;
    let top = y.floor() as u32;
    let signal = |x, y| {
        let pixel = frame.get_pixel(x, y);
        f32::from(pixel.0[0]) - f32::from(pixel.0[1])
    };
    let horizontal = x - left as f32;
    let vertical = y - top as f32;
    let upper = signal(left, top) * (1.0 - horizontal) + signal(left + 1, top) * horizontal;
    let lower = signal(left, top + 1) * (1.0 - horizontal) + signal(left + 1, top + 1) * horizontal;
    upper * (1.0 - vertical) + lower * vertical
}

fn module_position(size: usize, rotation: usize, row: usize, col: usize) -> (usize, usize) {
    match rotation {
        0 => (row, col),
        1 => (col, size - 1 - row),
        2 => (size - 1 - row, size - 1 - col),
        _ => (size - 1 - col, row),
    }
}

fn bit_at_rotation(
    correlations: &[f32],
    size: usize,
    rotation: usize,
    row: usize,
    col: usize,
) -> bool {
    let (source_row, source_col) = module_position(size, rotation, row, col);
    correlations[source_row * size + source_col] < 0.0
}

fn finder_hits(
    correlations: &[f32],
    size: usize,
    quiet: usize,
    rotation: usize,
    polarity: bool,
) -> u16 {
    let mut hits = 0;
    let symbol_size = size - 2 * quiet;
    for &(row_offset, col_offset) in &[(0, 0), (0, symbol_size - 7), (symbol_size - 7, 0)] {
        for row in 0..7 {
            for col in 0..7 {
                let actual = bit_at_rotation(
                    correlations,
                    size,
                    rotation,
                    quiet + row_offset + row,
                    quiet + col_offset + col,
                ) ^ polarity;
                if actual == FINDER[row][col] {
                    hits += 1;
                }
            }
        }
    }
    hits
}

fn align_grid(correlations: &[f32], size: usize) -> (u16, usize, usize, bool) {
    let max_quiet = MAX_QUIET_SCAN.min((size - 21) / 2);
    let mut best_hits = 0;
    let mut best = (0, 0, false);
    for rotation in 0..4 {
        for polarity in [false, true] {
            for quiet in 0..=max_quiet {
                let hits = finder_hits(correlations, size, quiet, rotation, polarity);
                if hits > best_hits {
                    best_hits = hits;
                    best = (quiet, rotation, polarity);
                }
            }
        }
    }
    (best_hits, best.0, best.1, best.2)
}

fn render_symbol(hypothesis: &Hypothesis) -> Option<GrayImage> {
    let symbol_size = hypothesis.modules.checked_sub(2 * hypothesis.quiet)?;
    if symbol_size < 21 {
        return None;
    }
    let output_modules = symbol_size + 2 * OUTPUT_QUIET_MODULES;
    let side = output_modules.checked_mul(OUTPUT_MODULE_PIXELS)?;
    let mut image = GrayImage::from_pixel(side as u32, side as u32, Luma([255]));
    for row in 0..symbol_size {
        for col in 0..symbol_size {
            let dark = bit_at_rotation(
                &hypothesis.correlations,
                hypothesis.modules,
                hypothesis.rotation,
                hypothesis.quiet + row,
                hypothesis.quiet + col,
            ) ^ hypothesis.polarity;
            if dark {
                let origin_y = (OUTPUT_QUIET_MODULES + row) * OUTPUT_MODULE_PIXELS;
                let origin_x = (OUTPUT_QUIET_MODULES + col) * OUTPUT_MODULE_PIXELS;
                for y in origin_y..origin_y + OUTPUT_MODULE_PIXELS {
                    for x in origin_x..origin_x + OUTPUT_MODULE_PIXELS {
                        image.put_pixel(x as u32, y as u32, Luma([0]));
                    }
                }
            }
        }
    }
    Some(image)
}

fn decode_standard_qr(image: GrayImage) -> Option<String> {
    let mut prepared = rqrr::PreparedImage::prepare(image);
    let grids = prepared.detect_grids();
    let mut decoded = None;
    for grid in grids {
        if let Ok((_, text)) = grid.decode() {
            if text.is_empty() || decoded.is_some() {
                return None;
            }
            decoded = Some(text);
        }
    }
    decoded
}
