use std::io;

use axum::{
    extract::rejection::PathRejection,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ServerError {
    #[error(transparent)]
    ValidationError(#[from] validator::ValidationErrors),

    #[error(transparent)]
    AxumPathRejection(#[from] PathRejection),

    #[error(transparent)]
    ReqwestError(#[from] reqwest::Error),

    #[error(transparent)]
    IOError(#[from] io::Error),

    #[error(transparent)]
    ParseURLError(#[from] url::ParseError),
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            ServerError::ValidationError(err) => {
                let error_message = err
                    .field_errors()
                    .iter()
                    .map(|(field, errors)| {
                        let error_messages: Vec<String> = errors
                            .iter()
                            .map(|error| error.message.clone().unwrap_or_default().to_string())
                            .collect();
                        format!("{}: {}", field, error_messages.join(", "))
                    })
                    .collect::<Vec<String>>()
                    .join("; ");
                (
                    StatusCode::BAD_REQUEST,
                    format!("Validation error: {}", error_message),
                )
            }
            ServerError::AxumPathRejection(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            ServerError::ReqwestError(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            ServerError::IOError(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            Self::ParseURLError(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };
        let body = Json(ErrorResponse {
            error: error_message,
        });
        (status, body).into_response()
    }
}
