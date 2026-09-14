use axum::{http::StatusCode, response::IntoResponse};

#[derive(Debug)]
pub enum AppError {
    BadRequest(String),
    Unauthorized(String),
    PermissionDenied(String),
    Unknown(color_eyre::eyre::Report),
}

impl<T> From<T> for AppError
where
    T: Into<color_eyre::eyre::Report>,
{
    fn from(value: T) -> Self {
        Self::Unknown(value.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        match self {
            AppError::BadRequest(message) => (StatusCode::BAD_REQUEST, message).into_response(),
            AppError::Unauthorized(message) => (StatusCode::UNAUTHORIZED, message).into_response(),
            AppError::PermissionDenied(message) => (StatusCode::FORBIDDEN, message).into_response(),
            AppError::Unknown(report) => {
                (StatusCode::INTERNAL_SERVER_ERROR, report.to_string()).into_response()
            }
        }
    }
}

pub type AppResult<T = ()> = Result<T, AppError>;

impl<T> Into<AppResult<T>> for AppError {
    fn into(self) -> AppResult<T> {
        Err(self)
    }
}
