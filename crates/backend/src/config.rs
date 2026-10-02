use std::{net::SocketAddr, path::PathBuf};

#[derive(Clone, Debug)]
pub struct Config {
    pub bind: SocketAddr,
    pub database_url: String,
    pub origin: String,
    pub cookie_secure: bool,
    pub model_dir: PathBuf,
}

impl Config {
    pub fn from_env() -> Result<Self, crate::StartupError> {
        let value = |key: &str, default: &str| match std::env::var(key) {
            Ok(value) => Ok(value),
            Err(std::env::VarError::NotPresent) => Ok(default.to_owned()),
            Err(_) => Err(crate::StartupError::Configuration),
        };
        let config = Self {
            bind: value("AEGIS_BIND", "127.0.0.1:3000")?
                .parse()
                .map_err(|_| crate::StartupError::Configuration)?,
            database_url: value("DATABASE_URL", "sqlite://data/aegis.db")?,
            origin: value("AEGIS_ORIGIN", "http://localhost:3000")?,
            cookie_secure: value("AEGIS_COOKIE_SECURE", "false")?
                .parse()
                .map_err(|_| crate::StartupError::Configuration)?,
            model_dir: value("AEGIS_MODEL_DIR", "models")?.into(),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), crate::StartupError> {
        let uri: axum::http::Uri = self
            .origin
            .parse()
            .map_err(|_| crate::StartupError::Configuration)?;
        let host = uri.host().ok_or(crate::StartupError::Configuration)?;
        let local = matches!(host, "localhost" | "127.0.0.1" | "[::1]");
        let scheme = uri.scheme_str().ok_or(crate::StartupError::Configuration)?;
        if !matches!(scheme, "http" | "https")
            || uri.path() != "/"
            || uri.query().is_some()
            || self.origin.ends_with('/')
            || (!local && (scheme != "https" || !self.cookie_secure))
            || !self.database_url.starts_with("sqlite:")
            || self.model_dir.as_os_str().is_empty()
        {
            return Err(crate::StartupError::Configuration);
        }
        Ok(())
    }
}
