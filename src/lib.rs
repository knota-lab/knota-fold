//! knota-fold — knowledge management platform backend.
//!
//! Selective allows for pedantic/nursery lints that are either:
//! - Documentation boilerplate (missing_errors_doc, doc_markdown, must_use_candidate)
//! - Stylistic preferences that don't affect correctness
//! - Unavoidable due to DB column types (i32/i64 vs usize/u64)

// ── Documentation / doc-comment lints ──────────────────────────────
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::too_long_first_doc_paragraph)]
#![allow(clippy::missing_fields_in_debug)]
// ── Attribute / struct style lints ─────────────────────────────────
#![allow(clippy::return_self_not_must_use)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::items_after_statements)]
// ── Cast lints — DB column types (i32/i64) vs Rust (usize/u64) ─────
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::cast_precision_loss)]
// ── Import / closure / format style lints ──────────────────────────
#![allow(clippy::redundant_closure_for_method_calls)]
#![allow(clippy::format_push_string)]
// ── Other style lints ──────────────────────────────────────────────
#![allow(clippy::too_many_lines)]
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::large_futures)]
#![allow(clippy::large_stack_arrays)]
#![allow(clippy::large_stack_frames)]

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
