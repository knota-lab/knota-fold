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
#![allow(clippy::used_underscore_binding)]
#![allow(clippy::items_after_statements)]
#![allow(clippy::unnecessary_struct_initialization)]
// ── Cast lints — DB column types (i32/i64) vs Rust (usize/u64) ─────
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::cast_precision_loss)]
// ── Import / closure / format style lints ──────────────────────────
#![allow(clippy::wildcard_imports)]
#![allow(clippy::redundant_closure_for_method_calls)]
#![allow(clippy::format_push_string)]
#![allow(clippy::implicit_clone)]
#![allow(clippy::match_same_arms)]
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::option_if_let_else)]
// ── Other style lints ──────────────────────────────────────────────
#![allow(clippy::trait_duplication_in_bounds)]
#![allow(clippy::needless_continue)]
#![allow(clippy::unreadable_literal)]
#![allow(clippy::unnested_or_patterns)]
#![allow(clippy::single_char_pattern)]
#![allow(clippy::stable_sort_primitive)]
#![allow(clippy::semicolon_if_nothing_returned)]
#![allow(clippy::large_futures)]
#![allow(clippy::large_stack_arrays)]
#![allow(clippy::large_stack_frames)]
#![allow(clippy::trivially_copy_pass_by_ref)]
#![allow(clippy::ref_option)]
#![allow(clippy::assigning_clones)]
#![allow(clippy::elidable_lifetime_names)]
#![allow(clippy::if_not_else)]
#![allow(clippy::needless_collect)]
#![allow(clippy::or_fun_call)]
#![allow(clippy::manual_let_else)]
#![allow(clippy::ignored_unit_patterns)]
#![allow(clippy::equatable_if_let)]
#![allow(clippy::format_collect)]
#![allow(clippy::explicit_iter_loop)]
#![allow(clippy::match_wildcard_for_single_variants)]
#![allow(clippy::manual_is_variant_and)]
#![allow(clippy::needless_for_each)]
#![allow(clippy::needless_raw_string_hashes)]
#![allow(clippy::unnecessary_literal_bound)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::unused_async)]
#![allow(clippy::useless_let_if_seq)]

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
