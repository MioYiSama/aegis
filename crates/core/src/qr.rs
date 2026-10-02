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
    if photo.width() < 3 || photo.height() < 3 {
        return Err(QrError::Unreadable);
    }
    if photo.width() > i32::MAX as u32 || photo.height() > i32::MAX as u32 {
        return Err(QrError::Unreadable);
    }
    let gray = grayscale_rec601(photo);
    let quads = detect_frame_quads(&gray);
    if quads.is_empty() {
        return Err(QrError::Unreadable);
    }
    let mut hypotheses = search_hypotheses(photo, &quads);
    hypotheses.sort_by(|left, right| right.hits.cmp(&left.hits));
    for hypothesis in hypotheses
        .iter()
        .filter(|hypothesis| hypothesis.hits >= ALIGNMENT_GATE_HITS)
        .take(5)
    {
        let Some(symbol) = render_symbol(hypothesis) else {
            continue;
        };
        if let Some(text) = decode_standard_qr(symbol) {
            return Ok(text);
        }
    }
    Err(QrError::Unreadable)
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
        let [red, green, blue] = source.0;
        let value = 0.299 * f64::from(red) + 0.587 * f64::from(green) + 0.114 * f64::from(blue);
        target.0[0] = value.round().clamp(0.0, 255.0) as u8;
    }
    gray
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
        percentile(&histogram, count, 50.0),
    )
}

fn detect_frame_quads(gray: &GrayImage) -> Vec<FrameQuad> {
    let (dark, middle) = sampled_quantiles(gray);
    let width = gray.width() as usize;
    let height = gray.height() as usize;
    let min_area = 0.002 * (width * height) as f64;
    let mut ring_like = Vec::new();
    let mut other = Vec::new();
    let mut seen: HashSet<[i32; 8]> = HashSet::new();

    for factor in [0.35, 0.6] {
        let threshold = dark + factor * (middle - dark);
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
            return Some(quad);
        }
    }
    None
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
    hits: u16,
    modules: usize,
    quiet: usize,
    rotation: usize,
    polarity: bool,
    correlations: Vec<f32>,
}

fn search_hypotheses(photo: &RgbImage, quads: &[FrameQuad]) -> Vec<Hypothesis> {
    let mut found = Vec::with_capacity(20);
    for frame_quad in quads {
        let mut stop = false;
        for modules in [37, 45, 29, 41, 33] {
            let Some(frame) = warp_frame(photo, &frame_quad.quad, modules, frame_quad.inset) else {
                continue;
            };
            if !frame_is_plausible(&frame, modules) {
                continue;
            }
            let correlations = module_correlations(&frame, modules);
            let (hits, quiet, rotation, polarity) = align_grid(&correlations, modules);
            found.push(Hypothesis {
                hits,
                modules,
                quiet,
                rotation,
                polarity,
                correlations,
            });
            if hits >= ALIGNMENT_GATE_HITS {
                stop = true;
                break;
            }
        }
        if stop {
            break;
        }
    }
    found
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

fn frame_is_plausible(frame: &RgbImage, modules: usize) -> bool {
    let gray = grayscale_rec601(frame);
    let side = frame.width() as usize;
    let offset = PAYLOAD_OFFSET;
    let edge = side - offset;

    let mean_band = |start: usize, end: usize| {
        let mut sum = 0_u64;
        let mut count = 0_u64;
        for y in start..end {
            for x in offset..edge {
                sum += u64::from(gray.get_pixel(x as u32, y as u32).0[0]);
                sum += u64::from(gray.get_pixel(x as u32, (side - end + y - start) as u32).0[0]);
                count += 2;
            }
        }
        for x in start..end {
            for y in offset..edge {
                sum += u64::from(gray.get_pixel(x as u32, y as u32).0[0]);
                sum += u64::from(gray.get_pixel((side - end + x - start) as u32, y as u32).0[0]);
                count += 2;
            }
        }
        sum as f64 / count as f64
    };

    let ring = mean_band(MARGIN_PIXELS + 2, MARGIN_PIXELS + BORDER_PIXELS - 2);
    let gap = mean_band(MARGIN_PIXELS + BORDER_PIXELS + 2, PAYLOAD_OFFSET - 2);
    let mut payload_sum = 0_u64;
    let payload_side = modules * MODULE_PIXELS;
    for y in offset..offset + payload_side {
        for x in offset..offset + payload_side {
            payload_sum += u64::from(gray.get_pixel(x as u32, y as u32).0[0]);
        }
    }
    let payload_mean = payload_sum as f64 / (payload_side * payload_side) as f64;
    ring + 15.0 < gap && ring + 15.0 < payload_mean
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

fn bit_at_rotation(
    correlations: &[f32],
    size: usize,
    rotation: usize,
    row: usize,
    col: usize,
) -> bool {
    let (source_row, source_col) = match rotation {
        0 => (row, col),
        1 => (col, size - 1 - row),
        2 => (size - 1 - row, size - 1 - col),
        _ => (size - 1 - col, row),
    };
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
