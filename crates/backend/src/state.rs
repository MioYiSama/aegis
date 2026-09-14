use color_eyre::eyre::*;
use toasty::Db;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
}

impl AppState {
    pub async fn new(database_url: &str) -> Result<Self> {
        let db = crate::db::connect(database_url).await?;
        crate::db::migrate(&db).await?;

        Ok(Self { db })
    }
}
