//! knota-fold — knowledge management platform backend.

#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
// loco-rs `Error` is intentionally large; box at API boundaries is not worth the churn.
#![allow(clippy::result_large_err)]

pub mod app;
pub mod app_logs;
pub mod config;
pub mod data;
pub mod error_info;
pub mod extractors;
pub mod initializers;
pub mod mailers;
pub mod middleware;
pub mod models;
pub mod modules;
pub mod services;
pub mod tasks;
pub mod utils;
pub mod views;
pub mod workers;
