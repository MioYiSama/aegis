use color_eyre::eyre::*;
use toasty::{Db, ModelSet};

pub mod user;

pub async fn connect() -> Result<Db> {
    let url = url_from_env()?;
    let db = Db::builder().models(models()).connect(&url).await?;
    Ok(db)
}

fn url_from_env() -> Result<String> {
    Ok(std::env::var("DATABASE_URL")?)
}

#[cfg(not(debug_assertions))]
static MIGRATIONS: toasty::migration::MigrationSet = toasty::embed_migrations!("../../toasty");

pub async fn migrate(#[allow(unused)] db: &Db) -> Result {
    #[cfg(not(debug_assertions))]
    MIGRATIONS.apply(db).await?;
    Ok(())
}

fn models() -> ModelSet {
    vec![user::models()]
        .into_iter()
        .flatten()
        .fold(ModelSet::new(), |mut models, model| {
            models.add(model);
            models
        })
}
