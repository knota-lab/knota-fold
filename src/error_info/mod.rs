//! Compile-time typed error declarations.
//!
//! `ErrorInfo` is an enum where each variant maps 1:1 to an HTTP status code.
//! Using `const` declarations ensures typo safety — a wrong code won't compile.
//!
//! `seed_error_codes` task scans all `**/error_info.rs` files and extracts
//! `ErrorInfo` constants for i18n seeding.

use axum::http::StatusCode;

#[derive(Debug, Clone, Copy)]
pub enum ErrorInfo {
    Unauthorized(&'static str, &'static str), // code, description
    Forbidden(&'static str, &'static str),
    NotFound(&'static str, &'static str),
    BadRequest(&'static str, &'static str),
    Conflict(&'static str, &'static str),
    Internal(&'static str, &'static str),
}

impl ErrorInfo {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Unauthorized(_, _) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_, _) => StatusCode::FORBIDDEN,
            Self::NotFound(_, _) => StatusCode::NOT_FOUND,
            Self::BadRequest(_, _) => StatusCode::BAD_REQUEST,
            Self::Conflict(_, _) => StatusCode::CONFLICT,
            Self::Internal(_, _) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Unauthorized(c, _)
            | Self::Forbidden(c, _)
            | Self::NotFound(c, _)
            | Self::BadRequest(c, _)
            | Self::Conflict(c, _)
            | Self::Internal(c, _) => c,
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::Unauthorized(_, d)
            | Self::Forbidden(_, d)
            | Self::NotFound(_, d)
            | Self::BadRequest(_, d)
            | Self::Conflict(_, d)
            | Self::Internal(_, d) => d,
        }
    }
}

pub mod api_key;
pub mod auth;
pub mod authz;
pub mod common;
pub mod dict;
pub mod file;
pub mod i18n;
pub mod role;
pub mod sys_config;
pub mod tenant;
pub mod upload;
pub mod user;
pub mod worker;
