use axum::http::StatusCode;
use loco_rs::controller::ErrorDetail;
use loco_rs::prelude::*;
use sea_orm::{DbErr, RuntimeErr};

// ─── Macro: DRY error logging with status-based level selection ────

/// Emit a tracing event with the caller's <file:line>.
///
/// Level selection: 5xx → `error!` (actionable, needs human attention),
/// 4xx → `warn!` (normal gatekeeping, worth monitoring for trends).
///
/// Supports optional extra key=value fields (e.g. `raw_error`).
///
/// Must be used INSIDE a `#[track_caller]` function to capture the
/// original business-logic caller.
#[macro_export]
macro_rules! log_error {
    // With extra key=value fields: log_error!(code, status, "msg", raw_error = %e)
    ($code:expr, $status:expr, $kind:expr, $($key:ident = $($val:tt)+),* $(,)?) => {
        let __status: u16 = $status;
        let __caller = ::std::panic::Location::caller();
        if __status >= 500 {
            ::tracing::error!(
                code = $code,
                status = __status,
                location = %format_args!(
                    "{}:{}:{}",
                    __caller.file(),
                    __caller.line(),
                    __caller.column()
                ),
                caller_file = __caller.file(),
                caller_line = __caller.line(),
                caller_column = __caller.column(),
                $($key = $($val)+,)*
                $kind,
            );
        } else {
            ::tracing::warn!(
                code = $code,
                status = __status,
                location = %format_args!(
                    "{}:{}:{}",
                    __caller.file(),
                    __caller.line(),
                    __caller.column()
                ),
                caller_file = __caller.file(),
                caller_line = __caller.line(),
                caller_column = __caller.column(),
                $($key = $($val)+,)*
                $kind,
            );
        }
    };
    // Without extra fields: log_error!(code, status, "msg")
    ($code:expr, $status:expr, $kind:expr $(,)?) => {
        log_error!($code, $status, $kind,)
    };
}

// ─── .db_err() — DB error auto-classification ──────────────────────
// Only for Result<T, DbErr>. Auto-classifies unique/not-found/FK/etc.

pub trait IntoAppError<T> {
    #[track_caller]
    fn db_err(self) -> Result<T>;
}

impl<T> IntoAppError<T> for Result<T, DbErr> {
    #[track_caller]
    fn db_err(self) -> Result<T> {
        match self {
            Ok(v) => Ok(v),
            Err(e) => {
                let (status, code, desc) = classify_db_err(&e);
                // raw_error follows the same level as the error itself:
                // 5xx → ERROR (visible in alerts), 4xx → WARN (visible in audits).
                log_error!(code, status.as_u16(), "DB error classified", raw_error = %e);
                Err(Error::CustomError(status, ErrorDetail::new(code, desc)))
            }
        }
    }
}

// ─── .loco_err() — generic fallback for non-DB errors ──────────────

pub trait IntoLocoResult<T> {
    fn loco_err(self) -> Result<T, Error>;
}

impl<T, E> IntoLocoResult<T> for Result<T, E>
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    fn loco_err(self) -> Result<T, Error> {
        self.map_err(|e| Error::Any(e.into()))
    }
}

/// Convert a bare `DbErr` into a classified `Error` (same logic as `.db_err()`).
///
/// Use when you need to extract the `Error` from a `DbErr` in a match arm
/// that has cleanup side-effects (e.g. S3 abort) and can't use `.db_err()?`.
///
/// ```ignore
/// match ctx.db.begin().await {
///     Ok(txn) => txn,
///     Err(e) => {
///         abort_multipart(...).await;  // cleanup
///         return Err(db_err_into(e));
///     }
/// }
/// ```
#[track_caller]
pub fn db_err_into(err: &DbErr) -> Error {
    let (status, code, desc) = classify_db_err(err);
    log_error!(code, status.as_u16(), "db error classified (bare)", raw_error = %err);
    Error::CustomError(status, ErrorDetail::new(code, desc))
}

fn classify_db_err(err: &DbErr) -> (StatusCode, &'static str, &'static str) {
    match err {
        DbErr::RecordNotFound(_) => {
            return (
                StatusCode::NOT_FOUND,
                crate::error_info::common::NOT_FOUND.code(),
                crate::error_info::common::NOT_FOUND.description(),
            )
        }
        DbErr::ConnectionAcquire(_) | DbErr::Conn(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                crate::error_info::common::DB_ERROR.code(),
                crate::error_info::common::DB_ERROR.description(),
            )
        }
        _ => {}
    }
    if let Some(sqlx_err) = extract_sqlx_error(err) {
        if let Some(db_err) = sqlx_err.as_database_error() {
            if let Some(code) = db_err.code() {
                match code.as_ref() {
                    // PostgreSQL: unique violation
                    "23505" |
                    // SQLite: SQLITE_CONSTRAINT_UNIQUE
                    "2067" |
                    // MySQL: duplicate entry
                    "1062" => {
                        return (
                            StatusCode::CONFLICT,
                            crate::error_info::common::DUPLICATE.code(),
                            crate::error_info::common::DUPLICATE.description(),
                        )
                    }
                    // PostgreSQL: foreign key violation
                    "23503" |
                    // SQLite: SQLITE_CONSTRAINT_FOREIGNKEY
                    "787" |
                    // MySQL: foreign key violation
                    "1452" => {
                        return (
                            StatusCode::BAD_REQUEST,
                            crate::error_info::common::INVALID_REFERENCE.code(),
                            crate::error_info::common::INVALID_REFERENCE.description(),
                        )
                    }
                    // PostgreSQL: not-null violation
                    "23502" => {
                        return (
                            StatusCode::BAD_REQUEST,
                            crate::error_info::common::MISSING_FIELD.code(),
                            crate::error_info::common::MISSING_FIELD.description(),
                        )
                    }
                    _ => {}
                }
            }
        }
        if matches!(sqlx_err, sqlx::Error::PoolTimedOut) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                crate::error_info::common::DB_ERROR.code(),
                crate::error_info::common::DB_ERROR.description(),
            );
        }
    }
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        crate::error_info::common::DB_ERROR.code(),
        crate::error_info::common::DB_ERROR.description(),
    )
}

const fn extract_sqlx_error(err: &DbErr) -> Option<&sqlx::Error> {
    match err {
        DbErr::Exec(RuntimeErr::SqlxError(e))
        | DbErr::Query(RuntimeErr::SqlxError(e))
        | DbErr::Conn(RuntimeErr::SqlxError(e)) => Some(e),
        _ => None,
    }
}

// ─── .model_err() — ModelResult<T> auto-classification ────────────
// Only for Result<T, ModelError>. Auto-classifies EntityNotFound /
// EntityAlreadyExists / DbErr / Any(DbErr).

pub trait IntoModelResult<T> {
    #[track_caller]
    fn model_err(self) -> Result<T>;
}

impl<T> IntoModelResult<T> for Result<T, loco_rs::model::ModelError> {
    #[track_caller]
    fn model_err(self) -> Result<T> {
        match self {
            Ok(v) => Ok(v),
            Err(loco_rs::model::ModelError::EntityNotFound) => {
                log_error!(
                    crate::error_info::common::NOT_FOUND.code(),
                    StatusCode::NOT_FOUND.as_u16(),
                    "ModelError::EntityNotFound"
                );
                Err(Error::CustomError(
                    StatusCode::NOT_FOUND,
                    ErrorDetail::new(
                        crate::error_info::common::NOT_FOUND.code(),
                        crate::error_info::common::NOT_FOUND.description(),
                    ),
                ))
            }
            Err(loco_rs::model::ModelError::EntityAlreadyExists) => {
                log_error!(
                    crate::error_info::common::DUPLICATE.code(),
                    StatusCode::CONFLICT.as_u16(),
                    "ModelError::EntityAlreadyExists"
                );
                Err(Error::CustomError(
                    StatusCode::CONFLICT,
                    ErrorDetail::new(
                        crate::error_info::common::DUPLICATE.code(),
                        crate::error_info::common::DUPLICATE.description(),
                    ),
                ))
            }
            Err(loco_rs::model::ModelError::DbErr(ref db_err)) => {
                let (status, code, desc) = classify_db_err(db_err);
                log_error!(
                    code,
                    status.as_u16(),
                    "ModelError::DbErr classified",
                    raw_error = %db_err
                );
                Err(Error::CustomError(status, ErrorDetail::new(code, desc)))
            }
            Err(loco_rs::model::ModelError::Any(inner)) => {
                // Try to downcast inner to DbErr for auto-classification
                if let Some(db_err) = inner.downcast_ref::<DbErr>() {
                    let (status, code, desc) = classify_db_err(db_err);
                    log_error!(
                        code,
                        status.as_u16(),
                        "ModelError::Any(DbErr) classified",
                        raw_error = %db_err
                    );
                    return Err(Error::CustomError(status, ErrorDetail::new(code, desc)));
                }
                log_error!(
                    "ModelError::Any",
                    StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                    "ModelError::Any(unknown)"
                );
                Err(Error::Any(inner))
            }
            Err(loco_rs::model::ModelError::Message(ref msg)) => {
                log_error!(
                    "common.bad_request",
                    StatusCode::BAD_REQUEST.as_u16(),
                    "ModelError::Message",
                    message = %msg
                );
                Err(Error::CustomError(
                    StatusCode::BAD_REQUEST,
                    ErrorDetail::new("common.bad_request", msg),
                ))
            }
            Err(e) => {
                log_error!(
                    "ModelError::Other",
                    StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                    "ModelError fallback",
                    error = %e
                );
                Err(Error::Any(e.into()))
            }
        }
    }
}

// Also impl for Result<T, DbErr> so .model_err() works on raw sea_orm calls
// (e.g. .insert().await, .exec().await, db.begin().await)
impl<T> IntoModelResult<T> for Result<T, DbErr> {
    #[track_caller]
    fn model_err(self) -> Result<T> {
        match self {
            Ok(v) => Ok(v),
            Err(e) => {
                let (status, code, desc) = classify_db_err(&e);
                log_error!(
                    code,
                    status.as_u16(),
                    "DbErr classified via .model_err()",
                    raw_error = %e
                );
                Err(Error::CustomError(status, ErrorDetail::new(code, desc)))
            }
        }
    }
}

// Identity impl: Result<T, Error> is already in the right shape — just pass through.
// This lets callers uniformly use `.model_err()` without caring whether the inner
// call already returns `loco_rs::Result<T>` (e.g. audit_service::log).
impl<T> IntoModelResult<T> for Result<T, Error> {
    #[track_caller]
    fn model_err(self) -> Self {
        match self {
            Ok(v) => Ok(v),
            Err(Error::DB(ref db_err)) => {
                let (status, code, desc) = classify_db_err(db_err);
                log_error!(
                    code,
                    status.as_u16(),
                    "loco Error::DB classified via .model_err()",
                    raw_error = %db_err
                );
                Err(Error::CustomError(status, ErrorDetail::new(code, desc)))
            }
            Err(e) => Err(e),
        }
    }
}

// ─── .err_info() — any Result → loco Result ────────────────────────

/// Convert any `Result<T, E>` into `loco_rs::Result<T>` using an `ErrorInfo`.
///
/// Replaces `.map_err(|_| from_info(...))`.
///
/// ```text
/// // Before
/// .map_err(|_| from_info(error_info::common::INVALID_UUID))?
///
/// // After
/// .err_info(error_info::common::INVALID_UUID)?
/// ```
pub trait ErrInto<T> {
    #[track_caller]
    fn err_info(self, info: crate::error_info::ErrorInfo) -> Result<T>;
}

impl<T, E> ErrInto<T> for Result<T, E> {
    #[track_caller]
    fn err_info(self, info: crate::error_info::ErrorInfo) -> Result<T> {
        self.map_or_else(|_| Err(crate::views::errors::from_info(info)), Ok)
    }
}

// ─── .err_info_with_desc() — with dynamic description ──────────────

/// Convert any `Result<T, E>` into `loco_rs::Result<T>` using an `ErrorInfo`
/// with a dynamically overridden description.
///
/// Replaces `.map_err(|e| from_info_with_desc(CONST, format!(...)))`.
pub trait ErrIntoDesc<T> {
    #[track_caller]
    fn err_info_with_desc(
        self,
        info: crate::error_info::ErrorInfo,
        desc: impl Into<String>,
    ) -> Result<T>;
}

impl<T, E> ErrIntoDesc<T> for Result<T, E> {
    #[track_caller]
    fn err_info_with_desc(
        self,
        info: crate::error_info::ErrorInfo,
        desc: impl Into<String>,
    ) -> Result<T> {
        self.map_or_else(
            |_| Err(crate::views::errors::from_info_with_desc(info, desc)),
            Ok,
        )
    }
}

// ─── .or_err() — Option → loco Result ──────────────────────────────

/// Convert `Option<T>` into `loco_rs::Result<T>` using an `ErrorInfo`.
///
/// Replaces `.ok_or_else(|| from_info(...))`.
///
/// ```text
/// // Before
/// .ok_or_else(|| from_info(error_info::role::NOT_FOUND))?
///
/// // After
/// .or_err(error_info::role::NOT_FOUND)?
/// ```
pub trait OptionErrInto<T> {
    #[track_caller]
    fn or_err(self, info: crate::error_info::ErrorInfo) -> Result<T>;
}

impl<T> OptionErrInto<T> for Option<T> {
    #[track_caller]
    fn or_err(self, info: crate::error_info::ErrorInfo) -> Result<T> {
        self.map_or_else(|| Err(crate::views::errors::from_info(info)), Ok)
    }
}
