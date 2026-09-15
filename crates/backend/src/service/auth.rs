use std::sync::LazyLock;

use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use base64::Engine;
use color_eyre::eyre::Result;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use validator::ValidationError;

use crate::model::UserRole;

static JWT_KEYS: LazyLock<(EncodingKey, DecodingKey)> = LazyLock::new(|| {
    let secret = std::env::var("JWT_SECRET").expect("JWT_SECRET must be specified");
    (
        EncodingKey::from_secret(secret.as_bytes()),
        DecodingKey::from_secret(secret.as_bytes()),
    )
});

static JWT_CONFIGS: LazyLock<(Header, Validation)> = LazyLock::new(|| {
    (
        Header::new(Algorithm::HS256),
        Validation::new(Algorithm::HS256),
    )
});

#[derive(Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: i64,
    pub role: UserRole,
}

pub fn generate_jwt(claims: &Claims) -> Result<String> {
    let token = jsonwebtoken::encode(&JWT_CONFIGS.0, claims, &JWT_KEYS.0)?;
    Ok(token)
}

pub fn parse_jwt(token: &str) -> Result<Claims> {
    let data = jsonwebtoken::decode(token, &JWT_KEYS.1, &JWT_CONFIGS.1)?;
    Ok(data.claims)
}

pub fn generate_token() -> String {
    let mut token = [0u8; 36];
    rand::rng().fill_bytes(&mut token);
    base64::engine::general_purpose::URL_SAFE.encode(token)
}

pub fn digest_token(token: &str) -> String {
    hex::encode(Sha256::digest(token))
}

pub fn hash_password(password: &str) -> Result<String> {
    let hash = Argon2::default().hash_password(password.as_bytes())?;
    Ok(hash.to_string())
}

pub fn verify_password(password_hash: &str, actual: &str) -> bool {
    Argon2::default()
        .verify_password(actual.as_bytes(), password_hash)
        .is_ok()
}

pub fn validate_role(role: &UserRole) -> Result<(), ValidationError> {
    if *role == UserRole::Admin {
        return Err(ValidationError::new("禁止注册管理员账号"));
    }

    Ok(())
}
