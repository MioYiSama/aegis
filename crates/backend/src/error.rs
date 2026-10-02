use axum::{
    extract::{FromRequest, Request},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Serialize, de::DeserializeOwned};
use utoipa::ToSchema;

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApiErrorCode {
    InvalidInput,
    Unauthenticated,
    Forbidden,
    NotFound,
    Conflict,
    WindowClosed,
    FactorUnavailable,
    Busy,
    Internal,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ApiError {
    pub code: ApiErrorCode,
    pub message: String,
    #[serde(skip)]
    #[schema(ignore)]
    pub status: StatusCode,
}
impl ApiError {
    fn new(code: ApiErrorCode, status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            code,
            status,
            message: message.into(),
        }
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(
            ApiErrorCode::InvalidInput,
            StatusCode::UNPROCESSABLE_ENTITY,
            message,
        )
    }
    pub fn unauthenticated() -> Self {
        Self::new(
            ApiErrorCode::Unauthenticated,
            StatusCode::UNAUTHORIZED,
            "Authentication required",
        )
    }
    pub fn forbidden() -> Self {
        Self::new(
            ApiErrorCode::Forbidden,
            StatusCode::FORBIDDEN,
            "Operation forbidden",
        )
    }
    pub fn not_found() -> Self {
        Self::new(
            ApiErrorCode::NotFound,
            StatusCode::NOT_FOUND,
            "Resource not found",
        )
    }
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(ApiErrorCode::Conflict, StatusCode::CONFLICT, message)
    }
    pub fn window_closed() -> Self {
        Self::new(
            ApiErrorCode::WindowClosed,
            StatusCode::CONFLICT,
            "Submission window closed",
        )
    }
    pub fn factor_unavailable() -> Self {
        Self::new(
            ApiErrorCode::FactorUnavailable,
            StatusCode::SERVICE_UNAVAILABLE,
            "Authentication factor unavailable",
        )
    }
    pub fn busy() -> Self {
        Self::new(
            ApiErrorCode::Busy,
            StatusCode::SERVICE_UNAVAILABLE,
            "Service busy",
        )
    }
    pub fn internal() -> Self {
        Self::new(
            ApiErrorCode::Internal,
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal service error",
        )
    }
    pub fn too_large() -> Self {
        Self::new(
            ApiErrorCode::InvalidInput,
            StatusCode::PAYLOAD_TOO_LARGE,
            "Media exceeds size limit",
        )
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, axum::Json(self)).into_response()
    }
}
impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        if let sqlx::Error::Database(db) = &error {
            if db
                .code()
                .and_then(|code| code.parse::<i32>().ok())
                .is_some_and(|code| matches!(code & 0xff, 5 | 6))
            {
                return Self::busy();
            }
            if db.is_unique_violation() {
                return Self::conflict("Resource already exists");
            }
        }
        if matches!(error, sqlx::Error::RowNotFound) {
            return Self::not_found();
        }
        if matches!(error, sqlx::Error::PoolTimedOut) {
            return Self::busy();
        }
        Self::internal()
    }
}

pub struct Json<T>(pub T);
impl<T> std::ops::Deref for Json<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}
impl<S, T> FromRequest<S> for Json<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;
    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::to_bytes(req.into_body(), 16 * 1024)
            .await
            .map_err(|_| ApiError::too_large())?;
        let _ = state;
        serde_json::from_slice(&bytes)
            .map(Self)
            .map_err(|_| ApiError::invalid("Invalid JSON fields"))
    }
}

pub struct Path<T>(pub T);
impl<S, T> axum::extract::FromRequestParts<S> for Path<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send,
{
    type Rejection = ApiError;
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> ApiResult<Self> {
        axum::extract::Path::<T>::from_request_parts(parts, state)
            .await
            .map(|p| Self(p.0))
            .map_err(|_| ApiError::invalid("Invalid path parameter"))
    }
}
