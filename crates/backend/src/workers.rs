use crate::{StartupError, error::ApiError};
use aegis_core::{
    face::{FaceEngine, FaceError, FaceTemplate},
    qr::{self, QrError},
};
use image::RgbImage;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
};
use tokio::sync::oneshot;

#[derive(Debug)]
pub enum WorkerError {
    Busy,
    Unavailable,
    Face(FaceError),
}
impl From<WorkerError> for ApiError {
    fn from(value: WorkerError) -> Self {
        match value {
            WorkerError::Busy => Self::busy(),
            WorkerError::Unavailable
            | WorkerError::Face(FaceError::InvalidModel | FaceError::Inference) => {
                Self::factor_unavailable()
            }
            WorkerError::Face(_) => Self::invalid("Face authentication failed"),
        }
    }
}

enum Job {
    Enroll {
        frames: [RgbImage; 3],
        reply: oneshot::Sender<Result<FaceTemplate, FaceError>>,
    },
    Verify {
        frames: [RgbImage; 3],
        template: FaceTemplate,
        reply: oneshot::Sender<Result<(), FaceError>>,
    },
    Qr {
        image: RgbImage,
        reply: oneshot::Sender<Result<String, QrError>>,
    },
}

#[derive(Clone)]
pub struct FaceWorkers {
    sender: mpsc::SyncSender<Job>,
}
impl FaceWorkers {
    pub async fn load(model_dir: PathBuf) -> Result<Self, StartupError> {
        tokio::task::spawn_blocking(move || {
            let engines = [
                FaceEngine::load(&model_dir).map_err(|_| StartupError::Models)?,
                FaceEngine::load(&model_dir).map_err(|_| StartupError::Models)?,
            ];
            let (sender, receiver) = mpsc::sync_channel::<Job>(16);
            let receiver = Arc::new(Mutex::new(receiver));
            for (index, mut engine) in engines.into_iter().enumerate() {
                let receiver = receiver.clone();
                std::thread::Builder::new()
                    .name(format!("aegis-inference-{index}"))
                    .spawn(move || {
                        loop {
                            let job = match receiver.lock() {
                                Ok(lock) => lock.recv(),
                                Err(_) => break,
                            };
                            let Ok(job) = job else { break };
                            match job {
                                Job::Enroll { frames, reply } => {
                                    let _ = reply
                                        .send(engine.enroll([&frames[0], &frames[1], &frames[2]]));
                                }
                                Job::Verify {
                                    frames,
                                    template,
                                    reply,
                                } => {
                                    let _ =
                                        reply.send(engine.verify(
                                            [&frames[0], &frames[1], &frames[2]],
                                            &template,
                                        ));
                                }
                                Job::Qr { image, reply } => {
                                    let _ = reply.send(qr::decode_chroma(&image));
                                }
                            }
                        }
                    })
                    .map_err(StartupError::Listener)?;
            }
            Ok(Self { sender })
        })
        .await
        .map_err(|_| StartupError::Models)?
    }
    fn submit(&self, job: Job) -> Result<(), WorkerError> {
        self.sender.try_send(job).map_err(|error| match error {
            mpsc::TrySendError::Full(_) => WorkerError::Busy,
            mpsc::TrySendError::Disconnected(_) => WorkerError::Unavailable,
        })
    }
    pub async fn enroll(&self, frames: [RgbImage; 3]) -> Result<FaceTemplate, WorkerError> {
        let (reply, result) = oneshot::channel();
        self.submit(Job::Enroll { frames, reply })?;
        result
            .await
            .map_err(|_| WorkerError::Unavailable)?
            .map_err(WorkerError::Face)
    }
    pub async fn verify(
        &self,
        frames: [RgbImage; 3],
        template: FaceTemplate,
    ) -> Result<(), WorkerError> {
        let (reply, result) = oneshot::channel();
        self.submit(Job::Verify {
            frames,
            template,
            reply,
        })?;
        result
            .await
            .map_err(|_| WorkerError::Unavailable)?
            .map_err(WorkerError::Face)
    }
    pub async fn decode_qr(&self, image: RgbImage) -> Result<Result<String, QrError>, WorkerError> {
        let (reply, result) = oneshot::channel();
        self.submit(Job::Qr { image, reply })?;
        result.await.map_err(|_| WorkerError::Unavailable)
    }
}
