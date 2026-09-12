use color_eyre::eyre::*;
use std::{ops::Deref, sync::Arc};
use toasty::Db;

#[derive(Clone)]
pub struct AppState {
    pub inner: Arc<AppStateInner>,
}

impl AppState {
    pub async fn new() -> Result<Self> {
        let inner = AppStateInner::new().await?;

        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    pub fn db(&self) -> Db {
        self.inner.db.clone()
    }
}

impl Deref for AppState {
    type Target = AppStateInner;

    fn deref(&self) -> &Self::Target {
        self.inner.deref()
    }
}

pub struct AppStateInner {
    pub db: Db,
}

impl AppStateInner {
    pub async fn new() -> Result<Self> {
        let db = crate::db::connect().await?;
        crate::db::migrate(&db).await?;

        Ok(Self { db })
    }
}
