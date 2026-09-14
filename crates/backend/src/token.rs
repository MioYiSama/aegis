use base64::Engine;
use color_eyre::eyre::*;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::LazyLock;

use crate::db::user::{User, UserRole};

#[derive(Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub iat: i64, // seconds
    pub exp: i64, // seconds
    pub role: UserRole,
}

impl From<&User> for Claims {
    fn from(user: &User) -> Self {
        let now = jiff::Timestamp::now().as_second();
        Self {
            sub: user.identity.clone(),
            iat: now,
            exp: now + 300,

            role: user.role.clone(),
        }
    }
}

static JWT_KEYS: LazyLock<(EncodingKey, DecodingKey)> = LazyLock::new(|| {
    let secret = std::env::var("JWT_SECRET").expect("JWT_SECRET is not specified");

    (
        EncodingKey::from_secret(secret.as_bytes()),
        DecodingKey::from_secret(secret.as_bytes()),
    )
});

static JWT_SETTINGS: LazyLock<(Header, Validation)> = LazyLock::new(|| {
    let algorithm = Algorithm::HS256;

    let header = Header::new(algorithm);

    let mut validation = Validation::new(algorithm);
    validation.leeway = 0;

    (header, validation)
});

pub fn generate_jwt(claims: impl Into<Claims>) -> Result<String> {
    let token = jsonwebtoken::encode(&JWT_SETTINGS.0, &claims.into(), &JWT_KEYS.0)?;
    Ok(token)
}

pub fn parse_jwt(token: &str) -> Result<Claims> {
    let data = jsonwebtoken::decode(token, &JWT_KEYS.1, &JWT_SETTINGS.1)?;
    Ok(data.claims)
}

pub fn generate_token() -> String {
    let mut token = [0u8; 32];
    rand::rng().fill_bytes(&mut token);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(token)
}

pub fn digest_token(token: &str) -> String {
    hex::encode(Sha256::digest(token))
}
