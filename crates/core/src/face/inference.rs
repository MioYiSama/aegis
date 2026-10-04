use std::path::Path;

use tract_onnx::prelude::*;

use super::FaceError;

/// A parsed ONNX graph with one reusable CPU execution state for its current shape.
///
/// YuNet's padded input dimensions vary with the source frame. The parsed graph and its
/// weights are kept here once; only the optimized plan/state is replaced when its shape
/// changes. Reusing the state also avoids allocating tract's per-node runtime state for
/// every frame on fixed-shape models.
pub(super) struct Network {
    model: InferenceModel,
    shape: [usize; 4],
    state: TypedSimpleState,
}

impl Network {
    pub(super) fn load(
        path: &Path,
        shape: [usize; 4],
        output_names: Option<&[&str]>,
    ) -> Result<Self, FaceError> {
        validate_shape(shape).map_err(|_| FaceError::InvalidModel)?;

        // YuNet's exported intermediate annotations describe 640x640, not the
        // current padded frame. Infer shapes from operators and concrete inputs
        // instead; output contracts are checked by the face pipeline.
        let mut model = tract_onnx::onnx()
            .with_ignore_value_info(true)
            .with_ignore_output_shapes(true)
            .model_for_path(path)
            .map_err(|_| FaceError::InvalidModel)?;
        if model
            .input_outlets()
            .map_err(|_| FaceError::InvalidModel)?
            .len()
            != 1
        {
            return Err(FaceError::InvalidModel);
        }

        if let Some(names) = output_names {
            if names.is_empty() {
                return Err(FaceError::InvalidModel);
            }
            model
                .select_outputs_by_name(names.iter().copied())
                .map_err(|_| FaceError::InvalidModel)?;
            if model
                .output_outlets()
                .map_err(|_| FaceError::InvalidModel)?
                .len()
                != names.len()
            {
                return Err(FaceError::InvalidModel);
            }
        }

        let state = compile_state(&model, shape).map_err(|_| FaceError::InvalidModel)?;
        Ok(Self {
            model,
            shape,
            state,
        })
    }

    pub(super) fn run(&mut self, input: Tensor) -> Result<TVec<TValue>, FaceError> {
        let shape = tensor_shape(&input).ok_or(FaceError::Inference)?;
        if shape != self.shape {
            let state = compile_state(&self.model, shape).map_err(|_| FaceError::Inference)?;
            self.state = state;
            self.shape = shape;
        }

        let inputs: TVec<TValue> = std::iter::once(input.into()).collect();
        self.state.run(inputs).map_err(|_| FaceError::Inference)
    }
}

fn validate_shape(shape: [usize; 4]) -> Result<(), ()> {
    if shape[0] == 1 && shape[1] == 3 && shape[2] > 0 && shape[3] > 0 {
        Ok(())
    } else {
        Err(())
    }
}

fn tensor_shape(input: &Tensor) -> Option<[usize; 4]> {
    if input.datum_type() != f32::datum_type() {
        return None;
    }
    let shape: [usize; 4] = input.shape().try_into().ok()?;
    validate_shape(shape).ok()?;
    Some(shape)
}

fn compile_state(model: &InferenceModel, shape: [usize; 4]) -> TractResult<TypedSimpleState> {
    let plan = model
        .clone()
        .with_input_fact(0, f32::fact(&shape).into())?
        .into_optimized()?
        .into_runnable()?;
    plan.spawn()
}
