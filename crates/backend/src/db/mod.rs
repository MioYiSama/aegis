use color_eyre::eyre::*;
use toasty::{Db, ModelSet};

pub mod auth;
pub mod user;

pub fn url_from_env() -> Option<String> {
    std::env::var("DATABASE_URL").ok()
}

pub async fn connect(url: &str) -> Result<Db> {
    let db = Db::builder().models(models()).connect(url).await?;
    Ok(db)
}

#[cfg(not(debug_assertions))]
static MIGRATIONS: toasty::migration::MigrationSet = toasty::embed_migrations!("../../toasty");

pub async fn migrate(#[allow(unused)] db: &Db) -> Result {
    #[cfg(not(debug_assertions))]
    MIGRATIONS.apply(db).await?;
    Ok(())
}

fn models() -> ModelSet {
    vec![auth::models(), user::models()]
        .into_iter()
        .flatten()
        .fold(ModelSet::new(), |mut models, model| {
            models.add(model);
            models
        })
}
