//! knota-fold — knowledge management platform backend.
//!
//! Remaining allows are for lints that are either:
//! - Documentation boilerplate too voluminous to maintain (`missing_errors_doc`, `missing_panics_doc`)
//! - Unavoidable DB column type mismatches (`cast_*` lints)
//! - Structural lints requiring deep refactoring (`too_many_lines`, `large_futures`, etc.)

#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]

pub mod app;
pub mod app_logs;
pub mod config;
pub mod controllers;
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
