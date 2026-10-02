use std::{fs, str::FromStr, time::Duration};

use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

use crate::config::Config;

/// Opens the application SQLite database and applies embedded migrations.
pub async fn connect(config: &Config) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::from_str(&config.database_url)?;

    if let Some(parent) = options.get_filename().parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(sqlx::Error::Io)?;
        }
    }

    let options = options
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .map_err(|error| sqlx::Error::Migrate(Box::new(error)))?;

    Ok(pool)
}
