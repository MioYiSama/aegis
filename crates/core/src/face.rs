//! Local YuNet/SFace identity verification and dual-MiniFASNet passive liveness.
//!
//! All OpenCV networks are owned by this engine and run on OpenCV's CPU backend.
//! The HTTP layer must keep this module behind its bounded blocking worker pool and
//! must never serialize `FaceTemplate` or `FrameAssessment`.

use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use image::RgbImage;
use opencv::{
    core::{self, Mat, MatTraitConst, Scalar, Size},
    dnn, imgproc, objdetect,
    prelude::{FaceDetectorYNTrait, FaceRecognizerSFTrait, FaceRecognizerSFTraitConst, NetTrait},
};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const MODEL_ID: &str = "yunet-2023mar_sface-2021dec_minifasnet-584d4421";
const YUNET_FILE: &str = "face_detection_yunet_2023mar.onnx";
const SFACE_FILE: &str = "face_recognition_sface_2021dec.onnx";
const MINIFAS_V2_FILE: &str = "2.7_80x80_MiniFASNetV2.onnx";
const MINIFAS_V1SE_FILE: &str = "4_0_0_80x80_MiniFASNetV1SE.onnx";
const CHECKSUMS_FILE: &str = "checksums.sha256";
pub const FACE_EMBEDDING_SIZE: usize = 128;
const LIVE_CLASS_COUNT: usize = 3;
const SFACE_COSINE_THRESHOLD: f64 = 0.363;
const LIVE_REAL_THRESHOLD: f64 = 0.90;

const MODEL_FILES: [&str; 4] = [YUNET_FILE, SFACE_FILE, MINIFAS_V2_FILE, MINIFAS_V1SE_FILE];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FaceError {
    #[error("input must contain exactly one detected face")]
    NotSingleFace,
    #[error("face failed passive liveness detection")]
    Spoof,
    #[error("face identity does not match the enrolled template")]
    IdentityMismatch,
    #[error("local face model bundle is invalid")]
    InvalidModel,
    #[error("local face inference failed")]
    Inference,
}

/// A model-bound, normalized SFace descriptor suitable for local persistence.
/// It is biometric data and must never be returned by an HTTP endpoint or logged.
#[derive(Clone)]
pub struct FaceTemplate {
    pub embedding: [f32; FACE_EMBEDDING_SIZE],
    pub model_id: String,
}

/// Per-frame SFace descriptor and the averaged probabilities from both liveness nets.
/// This is an internal algorithm result, not an HTTP response type.
#[derive(Clone, Copy)]
pub struct FrameAssessment {
    pub embedding: [f32; FACE_EMBEDDING_SIZE],
    pub live_probabilities: [f32; 3],
}

pub struct FaceEngine {
    detector: core::Ptr<objdetect::FaceDetectorYN>,
    recognizer: core::Ptr<objdetect::FaceRecognizerSF>,
    minifas_v2: dnn::Net,
    minifas_v1se: dnn::Net,
}

struct FrameWork {
    assessment: FrameAssessment,
    liveness_pass: bool,
}

struct ModelChecksums {
    by_model: [String; 4],
}

impl FaceEngine {
    /// Load and validate the four pinned local models, then perform real CPU forwards
    /// through YuNet (320x320), SFace (112x112), and both MiniFASNets (80x80).
    pub fn load(model_dir: &Path) -> Result<Self, FaceError> {
        let checksums = read_model_checksums(model_dir)?;
        for (index, filename) in MODEL_FILES.iter().enumerate() {
            let path = model_dir.join(filename);
            let digest = sha256_file(&path)?;
            if digest != checksums.by_model[index] {
                return Err(FaceError::InvalidModel);
            }
        }

        let yunet_path = model_path_string(model_dir, YUNET_FILE)?;
        let sface_path = model_path_string(model_dir, SFACE_FILE)?;
        let minifas_v2_path = model_path_string(model_dir, MINIFAS_V2_FILE)?;
        let minifas_v1se_path = model_path_string(model_dir, MINIFAS_V1SE_FILE)?;

        let mut detector = objdetect::FaceDetectorYN::create(
            &yunet_path,
            "",
            Size::new(320, 320),
            0.9,
            0.3,
            5000,
            dnn::DNN_BACKEND_OPENCV,
            dnn::DNN_TARGET_CPU,
        )
        .map_err(|_| FaceError::InvalidModel)?;
        let mut recognizer = objdetect::FaceRecognizerSF::create(
            &sface_path,
            "",
            dnn::DNN_BACKEND_OPENCV,
            dnn::DNN_TARGET_CPU,
        )
        .map_err(|_| FaceError::InvalidModel)?;
        let mut minifas_v2 = dnn::read_net_from_onnx(minifas_v2_path.as_str())
            .map_err(|_| FaceError::InvalidModel)?;
        let mut minifas_v1se = dnn::read_net_from_onnx(minifas_v1se_path.as_str())
            .map_err(|_| FaceError::InvalidModel)?;
        for network in [&mut minifas_v2, &mut minifas_v1se] {
            network
                .set_preferable_backend(dnn::DNN_BACKEND_OPENCV)
                .map_err(|_| FaceError::InvalidModel)?;
            network
                .set_preferable_target(dnn::DNN_TARGET_CPU)
                .map_err(|_| FaceError::InvalidModel)?;
        }

        // A no-face result is expected here; the detector call itself performs YuNet's
        // actual 320x320 inference and catches incompatible/truncated models at startup.
        let detector_input =
            filled_bgr_mat(320, 320, 127.0).map_err(|_| FaceError::InvalidModel)?;
        detector
            .set_input_size(Size::new(320, 320))
            .map_err(|_| FaceError::InvalidModel)?;
        let mut detections = Mat::default();
        let detected = detector
            .detect(&detector_input, &mut detections)
            .map_err(|_| FaceError::InvalidModel)?;
        if detected < 0 {
            return Err(FaceError::InvalidModel);
        }

        // SFace's feature forward is evaluated on an aligned-size BGR tensor.
        let sface_input = filled_bgr_mat(112, 112, 127.0).map_err(|_| FaceError::InvalidModel)?;
        let mut sface_output = Mat::default();
        recognizer
            .feature(&sface_input, &mut sface_output)
            .map_err(|_| FaceError::InvalidModel)?;
        copy_embedding(&sface_output).map_err(|_| FaceError::InvalidModel)?;

        // DNN blob conversion explicitly retains BGR and raw 0..255 values. A gray
        // value of 127 checks startup's model input without applying demo normalization.
        let live_probe = filled_bgr_mat(80, 80, 127.0).map_err(|_| FaceError::InvalidModel)?;
        let live_input = liveness_blob(&live_probe).map_err(|_| FaceError::InvalidModel)?;
        validate_liveness_blob(&live_input).map_err(|_| FaceError::InvalidModel)?;
        for network in [&mut minifas_v2, &mut minifas_v1se] {
            let logits =
                run_liveness_net(network, &live_input).map_err(|_| FaceError::InvalidModel)?;
            softmax(logits).map_err(|_| FaceError::InvalidModel)?;
        }

        Ok(Self {
            detector,
            recognizer,
            minifas_v2,
            minifas_v1se,
        })
    }

    /// Detect, align, extract and normalize one face embedding, and run both liveness
    /// models. Policy thresholds are deliberately applied only by `enroll`/`verify`.
    pub fn assess_frame(&mut self, frame: &RgbImage) -> Result<FrameAssessment, FaceError> {
        self.assess_frame_internal(frame)
            .map(|work| work.assessment)
    }

    /// Register three live frames after pairwise SFace consistency checks.
    pub fn enroll(&mut self, frames: [&RgbImage; 3]) -> Result<FaceTemplate, FaceError> {
        let assessments = [
            self.assess_frame_internal(frames[0])?,
            self.assess_frame_internal(frames[1])?,
            self.assess_frame_internal(frames[2])?,
        ];
        require_live_frames(&assessments)?;

        for left in 0..assessments.len() {
            for right in (left + 1)..assessments.len() {
                if cosine(
                    &assessments[left].assessment.embedding,
                    &assessments[right].assessment.embedding,
                ) < SFACE_COSINE_THRESHOLD
                {
                    return Err(FaceError::IdentityMismatch);
                }
            }
        }

        let mut embedding = [0.0; FACE_EMBEDDING_SIZE];
        for assessment in &assessments {
            for (mean, value) in embedding.iter_mut().zip(&assessment.assessment.embedding) {
                *mean += value / assessments.len() as f32;
            }
        }
        normalize_embedding(&mut embedding).ok_or(FaceError::Inference)?;
        Ok(FaceTemplate {
            embedding,
            model_id: MODEL_ID.to_owned(),
        })
    }

    /// Verify three live frames against a template generated by this exact model set.
    pub fn verify(
        &mut self,
        frames: [&RgbImage; 3],
        template: &FaceTemplate,
    ) -> Result<(), FaceError> {
        if template.model_id != MODEL_ID || !valid_normalized_embedding(&template.embedding) {
            return Err(FaceError::InvalidModel);
        }
        let assessments = [
            self.assess_frame_internal(frames[0])?,
            self.assess_frame_internal(frames[1])?,
            self.assess_frame_internal(frames[2])?,
        ];
        require_live_frames(&assessments)?;
        if assessments.iter().any(|assessment| {
            cosine(&template.embedding, &assessment.assessment.embedding) < SFACE_COSINE_THRESHOLD
        }) {
            return Err(FaceError::IdentityMismatch);
        }
        Ok(())
    }

    fn assess_frame_internal(&mut self, frame: &RgbImage) -> Result<FrameWork, FaceError> {
        let image_width = i32::try_from(frame.width()).map_err(|_| FaceError::Inference)?;
        let image_height = i32::try_from(frame.height()).map_err(|_| FaceError::Inference)?;
        if image_width < 2 || image_height < 2 || frame.as_raw().is_empty() {
            return Err(FaceError::NotSingleFace);
        }
        let bgr = rgb_image_to_bgr_mat(frame)?;
        self.detector
            .set_input_size(Size::new(image_width, image_height))
            .map_err(|_| FaceError::Inference)?;
        let mut detections = Mat::default();
        let count = self
            .detector
            .detect(&bgr, &mut detections)
            .map_err(|_| FaceError::Inference)?;
        if count < 0 {
            return Err(FaceError::Inference);
        }
        if detections.rows() != 1 {
            return Err(FaceError::NotSingleFace);
        }
        if detections.dims() != 2
            || detections.rows() != 1
            || detections.cols() != 15
            || detections.typ() != core::CV_32FC1
        {
            return Err(FaceError::Inference);
        }
        let mut bbox_xywh = [0.0_f32; 4];
        for (index, value) in bbox_xywh.iter_mut().enumerate() {
            *value = *detections
                .at_2d::<f32>(0, index as i32)
                .map_err(|_| FaceError::Inference)?;
        }
        if bbox_xywh.iter().any(|value| !value.is_finite())
            || bbox_xywh[2] <= 0.0
            || bbox_xywh[3] <= 0.0
        {
            return Err(FaceError::Inference);
        }

        let mut aligned = Mat::default();
        self.recognizer
            .align_crop(&bgr, &detections, &mut aligned)
            .map_err(|_| FaceError::Inference)?;
        let mut feature = Mat::default();
        self.recognizer
            .feature(&aligned, &mut feature)
            .map_err(|_| FaceError::Inference)?;
        let embedding = copy_normalized_embedding(&feature)?;

        let input_v2 = preprocess_liveness_crop(&bgr, bbox_xywh, 2.7)?;
        let logits_v2 = run_liveness_net(&mut self.minifas_v2, &input_v2)?;
        let input_v1se = preprocess_liveness_crop(&bgr, bbox_xywh, 4.0)?;
        let logits_v1se = run_liveness_net(&mut self.minifas_v1se, &input_v1se)?;
        let probabilities_v2 = softmax(logits_v2)?;
        let probabilities_v1se = softmax(logits_v1se)?;
        let mut live_probabilities = [0.0; LIVE_CLASS_COUNT];
        let mut combined_probabilities = [0.0; LIVE_CLASS_COUNT];
        for index in 0..LIVE_CLASS_COUNT {
            let combined =
                (f64::from(probabilities_v2[index]) + f64::from(probabilities_v1se[index])) / 2.0;
            if !combined.is_finite() {
                return Err(FaceError::Inference);
            }
            combined_probabilities[index] = combined;
            live_probabilities[index] = combined as f32;
        }
        let liveness_pass = first_argmax(&combined_probabilities) == 1
            && combined_probabilities[1] >= LIVE_REAL_THRESHOLD;

        Ok(FrameWork {
            assessment: FrameAssessment {
                embedding,
                live_probabilities,
            },
            liveness_pass,
        })
    }
}

// The preparation script pins upstream downloads; this manifest lets runtime verify local
// bytes offline while also permitting the documented fixed-batch conversion path.
fn read_model_checksums(model_dir: &Path) -> Result<ModelChecksums, FaceError> {
    let manifest =
        File::open(model_dir.join(CHECKSUMS_FILE)).map_err(|_| FaceError::InvalidModel)?;
    let mut reader = BufReader::new(manifest);
    let mut by_model: [String; 4] = std::array::from_fn(|_| String::new());
    let mut line = String::new();
    loop {
        line.clear();
        let bytes = reader
            .read_line(&mut line)
            .map_err(|_| FaceError::InvalidModel)?;
        if bytes == 0 {
            break;
        }
        let row = line.strip_suffix('\n').unwrap_or(&line);
        let row = row.strip_suffix('\r').unwrap_or(row);
        let (digest, filename) = row.split_once("  ").ok_or(FaceError::InvalidModel)?;
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || filename.bytes().any(|byte| byte.is_ascii_whitespace())
        {
            return Err(FaceError::InvalidModel);
        }
        let index = MODEL_FILES
            .iter()
            .position(|expected| *expected == filename)
            .ok_or(FaceError::InvalidModel)?;
        if !by_model[index].is_empty() {
            return Err(FaceError::InvalidModel);
        }
        by_model[index] = digest.to_owned();
    }
    if by_model.iter().any(String::is_empty) {
        return Err(FaceError::InvalidModel);
    }
    Ok(ModelChecksums { by_model })
}

fn sha256_file(path: &Path) -> Result<String, FaceError> {
    let file = File::open(path).map_err(|_| FaceError::InvalidModel)?;
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|_| FaceError::InvalidModel)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut hex, "{byte:02x}").map_err(|_| FaceError::InvalidModel)?;
    }
    Ok(hex)
}

fn model_path_string(model_dir: &Path, filename: &str) -> Result<String, FaceError> {
    // OpenCV's generated API accepts UTF-8 path strings. Non-UTF-8 model directories
    // fail closed instead of silently selecting a different file or fallback.
    model_dir
        .join(filename)
        .into_os_string()
        .into_string()
        .map_err(|_| FaceError::InvalidModel)
}

fn filled_bgr_mat(width: i32, height: i32, value: f64) -> opencv::Result<Mat> {
    Mat::new_rows_cols_with_default(height, width, core::CV_8UC3, Scalar::all(value))
}

fn rgb_image_to_bgr_mat(frame: &RgbImage) -> Result<Mat, FaceError> {
    let rows = i32::try_from(frame.height()).map_err(|_| FaceError::Inference)?;
    let rgb_row = Mat::from_slice(frame.as_raw()).map_err(|_| FaceError::Inference)?;
    let rgb = rgb_row.reshape(3, rows).map_err(|_| FaceError::Inference)?;
    let mut bgr = Mat::default();
    imgproc::cvt_color(
        &rgb,
        &mut bgr,
        imgproc::COLOR_RGB2BGR,
        0,
        core::AlgorithmHint::ALGO_HINT_DEFAULT,
    )
    .map_err(|_| FaceError::Inference)?;
    Ok(bgr)
}

fn preprocess_liveness_crop(
    bgr: &Mat,
    bbox: [f32; 4],
    requested_scale: f64,
) -> Result<Mat, FaceError> {
    let source_width = bgr.cols();
    let source_height = bgr.rows();
    let [x, y, box_width, box_height] = bbox.map(f64::from);
    let source_width_f64 = f64::from(source_width);
    let source_height_f64 = f64::from(source_height);
    let scale = (((source_height_f64 - 1.0) / box_height)
        .min((source_width_f64 - 1.0) / box_width))
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

    // Match the pinned test.py boundary behavior: translate the opposite edge to
    // preserve the expanded crop size before clipping the far edge to src-1.
    if left < 0.0 {
        right -= left;
        left = 0.0;
    }
    if top < 0.0 {
        bottom -= top;
        top = 0.0;
    }
    if right > source_width_f64 - 1.0 {
        left -= right - source_width_f64 + 1.0;
        right = source_width_f64 - 1.0;
    }
    if bottom > source_height_f64 - 1.0 {
        top -= bottom - source_height_f64 + 1.0;
        bottom = source_height_f64 - 1.0;
    }

    if ![left, top, right, bottom]
        .iter()
        .all(|value| value.is_finite())
    {
        return Err(FaceError::Inference);
    }
    // Float-to-int casts truncate toward zero, matching Python's int() after the
    // boundary logic has made all valid coordinates non-negative.
    let x0 = left as i32;
    let y0 = top as i32;
    let x1 = right as i32;
    let y1 = bottom as i32;
    if x0 < 0 || y0 < 0 || x1 < x0 || y1 < y0 || x1 >= source_width || y1 >= source_height {
        return Err(FaceError::Inference);
    }
    let roi = Mat::roi(bgr, core::Rect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1))
        .map_err(|_| FaceError::Inference)?;
    let mut resized = Mat::default();
    imgproc::resize(
        &roi,
        &mut resized,
        Size::new(80, 80),
        0.0,
        0.0,
        imgproc::INTER_LINEAR,
    )
    .map_err(|_| FaceError::Inference)?;
    let input = liveness_blob(&resized)?;
    validate_liveness_blob(&input)?;
    Ok(input)
}

fn liveness_blob(frame: &Mat) -> Result<Mat, FaceError> {
    dnn::blob_from_image(
        frame,
        1.0,
        Size::new(80, 80),
        Scalar::all(0.0),
        false,
        false,
        core::CV_32F,
    )
    .map_err(|_| FaceError::Inference)
}

fn validate_liveness_blob(input: &Mat) -> Result<(), FaceError> {
    let shape = input.mat_size();
    if input.typ() != core::CV_32FC1 || &*shape != [1, 3, 80, 80] {
        return Err(FaceError::Inference);
    }
    Ok(())
}

fn run_liveness_net(
    network: &mut dnn::Net,
    input: &Mat,
) -> Result<[f32; LIVE_CLASS_COUNT], FaceError> {
    validate_liveness_blob(input)?;
    network
        .set_input_def(input)
        .map_err(|_| FaceError::Inference)?;
    let output = network
        .forward_single_def()
        .map_err(|_| FaceError::Inference)?;
    if output.dims() != 2
        || output.rows() != 1
        || output.cols() != LIVE_CLASS_COUNT as i32
        || output.typ() != core::CV_32FC1
    {
        return Err(FaceError::Inference);
    }
    let mut logits = [0.0; LIVE_CLASS_COUNT];
    for (index, logit) in logits.iter_mut().enumerate() {
        *logit = *output
            .at_2d::<f32>(0, index as i32)
            .map_err(|_| FaceError::Inference)?;
    }
    if logits.iter().any(|logit| !logit.is_finite()) {
        return Err(FaceError::Inference);
    }
    Ok(logits)
}

fn softmax(logits: [f32; LIVE_CLASS_COUNT]) -> Result<[f32; LIVE_CLASS_COUNT], FaceError> {
    if logits.iter().any(|logit| !logit.is_finite()) {
        return Err(FaceError::Inference);
    }
    let maximum = logits.into_iter().fold(f32::NEG_INFINITY, f32::max);
    let mut probabilities = [0.0; LIVE_CLASS_COUNT];
    let mut denominator = 0.0;
    for (index, logit) in logits.into_iter().enumerate() {
        probabilities[index] = (logit - maximum).exp();
        denominator += probabilities[index];
    }
    if !denominator.is_finite() || denominator <= 0.0 {
        return Err(FaceError::Inference);
    }
    for probability in &mut probabilities {
        *probability /= denominator;
    }
    Ok(probabilities)
}

fn copy_embedding(feature: &Mat) -> Result<[f32; FACE_EMBEDDING_SIZE], FaceError> {
    if feature.dims() != 2
        || feature.rows() != 1
        || feature.cols() != FACE_EMBEDDING_SIZE as i32
        || feature.typ() != core::CV_32FC1
    {
        return Err(FaceError::Inference);
    }
    let mut embedding = [0.0; FACE_EMBEDDING_SIZE];
    for (index, value) in embedding.iter_mut().enumerate() {
        *value = *feature
            .at_2d::<f32>(0, index as i32)
            .map_err(|_| FaceError::Inference)?;
        if !value.is_finite() {
            return Err(FaceError::Inference);
        }
    }
    Ok(embedding)
}

fn copy_normalized_embedding(feature: &Mat) -> Result<[f32; FACE_EMBEDDING_SIZE], FaceError> {
    let mut embedding = copy_embedding(feature)?;
    normalize_embedding(&mut embedding).ok_or(FaceError::Inference)?;
    Ok(embedding)
}

fn normalize_embedding(embedding: &mut [f32; FACE_EMBEDDING_SIZE]) -> Option<()> {
    let norm_squared = embedding
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>();
    if !norm_squared.is_finite() || norm_squared <= f64::MIN_POSITIVE {
        return None;
    }
    let inverse_norm = norm_squared.sqrt().recip() as f32;
    if !inverse_norm.is_finite() || inverse_norm <= 0.0 {
        return None;
    }
    for value in embedding {
        *value *= inverse_norm;
        if !value.is_finite() {
            return None;
        }
    }
    Some(())
}

fn valid_normalized_embedding(embedding: &[f32; FACE_EMBEDDING_SIZE]) -> bool {
    if embedding.iter().any(|value| !value.is_finite()) {
        return false;
    }
    let norm_squared = embedding
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>();
    (norm_squared - 1.0).abs() <= 1.0e-3
}

fn cosine(left: &[f32; FACE_EMBEDDING_SIZE], right: &[f32; FACE_EMBEDDING_SIZE]) -> f64 {
    left.iter()
        .zip(right)
        .map(|(left, right)| f64::from(*left) * f64::from(*right))
        .sum()
}

fn require_live_frames(assessments: &[FrameWork; 3]) -> Result<(), FaceError> {
    if assessments
        .iter()
        .any(|assessment| !assessment.liveness_pass)
    {
        return Err(FaceError::Spoof);
    }
    Ok(())
}

fn first_argmax(values: &[f64; LIVE_CLASS_COUNT]) -> usize {
    let mut best = 0;
    for index in 1..LIVE_CLASS_COUNT {
        if values[index] > values[best] {
            best = index;
        }
    }
    best
}
