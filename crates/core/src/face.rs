//! Local YuNet/SFace identity verification and dual-MiniFASNet passive liveness.
//!
//! All networks run locally on tract's pure Rust CPU backend.
//! The HTTP layer must keep this module behind its bounded blocking worker pool and
//! must never serialize `FaceTemplate` or `FrameAssessment`.

use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use image::RgbImage;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tract_onnx::prelude::*;

mod inference;
mod processing;

use inference::Network;
use processing::{align_face, decode_detections, detector_input, liveness_input};

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
    detector: Network,
    recognizer: Network,
    minifas_v2: Network,
    minifas_v1se: Network,
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

        const DETECTOR_OUTPUTS: [&str; 12] = [
            "cls_8", "cls_16", "cls_32", "obj_8", "obj_16", "obj_32", "bbox_8", "bbox_16",
            "bbox_32", "kps_8", "kps_16", "kps_32",
        ];
        let mut detector = Network::load(
            &model_dir.join(YUNET_FILE),
            [1, 3, 320, 320],
            Some(&DETECTOR_OUTPUTS),
        )?;
        let mut recognizer = Network::load(&model_dir.join(SFACE_FILE), [1, 3, 112, 112], None)?;
        let mut minifas_v2 = Network::load(&model_dir.join(MINIFAS_V2_FILE), [1, 3, 80, 80], None)?;
        let mut minifas_v1se =
            Network::load(&model_dir.join(MINIFAS_V1SE_FILE), [1, 3, 80, 80], None)?;

        // Real forwards catch incompatible models before accepting authentication work.
        let probe = RgbImage::from_pixel(320, 320, image::Rgb([127; 3]));
        let detections = detector
            .run(detector_input(&probe).map_err(|_| FaceError::InvalidModel)?)
            .map_err(|_| FaceError::InvalidModel)?;
        match decode_detections(&detections, 320, 320) {
            Ok(_) | Err(FaceError::NotSingleFace) => {}
            Err(_) => return Err(FaceError::InvalidModel),
        }
        let feature = recognizer
            .run(gray_input(112).map_err(|_| FaceError::InvalidModel)?)
            .map_err(|_| FaceError::InvalidModel)?;
        copy_output::<FACE_EMBEDDING_SIZE>(&feature).map_err(|_| FaceError::InvalidModel)?;
        for network in [&mut minifas_v2, &mut minifas_v1se] {
            let output = network
                .run(gray_input(80).map_err(|_| FaceError::InvalidModel)?)
                .map_err(|_| FaceError::InvalidModel)?;
            let logits =
                copy_output::<LIVE_CLASS_COUNT>(&output).map_err(|_| FaceError::InvalidModel)?;
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
        if frame.width() < 2 || frame.height() < 2 || frame.as_raw().is_empty() {
            return Err(FaceError::NotSingleFace);
        }
        let detections = self.detector.run(detector_input(frame)?)?;
        let face = decode_detections(&detections, frame.width(), frame.height())?;
        let feature = self.recognizer.run(align_face(frame, &face)?)?;
        let mut embedding = copy_output::<FACE_EMBEDDING_SIZE>(&feature)?;
        normalize_embedding(&mut embedding).ok_or(FaceError::Inference)?;

        let output_v2 = self
            .minifas_v2
            .run(liveness_input(frame, face.bbox, 2.7)?)?;
        let logits_v2 = copy_output::<LIVE_CLASS_COUNT>(&output_v2)?;
        let output_v1se = self
            .minifas_v1se
            .run(liveness_input(frame, face.bbox, 4.0)?)?;
        let logits_v1se = copy_output::<LIVE_CLASS_COUNT>(&output_v1se)?;
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

// The preparation script pins upstream downloads; this manifest lets runtime
// verify the trusted local bundle offline before importing any network.
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

fn gray_input(size: usize) -> Result<Tensor, FaceError> {
    let mut input = Tensor::zero::<f32>(&[1, 3, size, size]).map_err(|_| FaceError::Inference)?;
    input
        .try_as_plain_ram_mut()
        .and_then(|mut view| {
            view.as_slice_mut::<f32>()?.fill(127.0);
            Ok(())
        })
        .map_err(|_| FaceError::Inference)?;
    Ok(input)
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

fn copy_output<const N: usize>(outputs: &[TValue]) -> Result<[f32; N], FaceError> {
    if outputs.len() != 1 || outputs[0].shape() != [1, N] {
        return Err(FaceError::Inference);
    }
    let values = outputs[0]
        .try_as_plain_ram()
        .and_then(|view| view.as_slice::<f32>())
        .map_err(|_| FaceError::Inference)?;
    if values.iter().any(|value| !value.is_finite()) {
        return Err(FaceError::Inference);
    }
    values.try_into().map_err(|_| FaceError::Inference)
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
