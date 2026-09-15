use color_eyre::eyre::*;
use toasty::Db;

use crate::model;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
}

pub async fn connect_db() -> Result<Db> {
    let url = std::env::var("DATABASE_URL")?;
    let db = Db::builder().models(model::models()).connect(&url).await?;
    Ok(db)
}

impl AppState {
    pub async fn new() -> Result<Self> {
        Ok(Self {
            db: connect_db().await?,
        })
    }
}
