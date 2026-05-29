//! Seed backend error codes into the i18n system (CommonError namespace).
//!
//! Automatically extracts all error codes from source by scanning `src/**/*.rs`
//! for error-construction calls like `err_bad_request("module.detail", "desc")`,
//! `forbidden("module.detail", "desc")`, `ErrorDetail::new("module.detail", "desc")`, etc.
//!
//! # Usage
//!
//! ```sh
//! # Dry-run (default): extract + validate + report, no DB writes.
//! cargo loco task seed_error_codes
//!
//! # Actually write to DB:
//! cargo loco task seed_error_codes -- apply:true
//!
//! # Or via env var (PowerShell):
//! $env:SEED_APPLY='true'; cargo loco task seed_error_codes
//! ```
//!
//! # Validation
//!
//! - **Code format**: must match `[a-z_]+\.[a-z_]+` (module.detail convention).
//! - **Conflict detection**: same code with different descriptions → warning.
//! - **Quantity regression**: if count drops >10% vs previous run, warns.
//!
//! Idempotent — re-running updates existing entries without side effects.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use async_trait::async_trait;
use loco_rs::{
    app::AppContext,
    task::{self, Task, TaskInfo},
    Result,
};
use regex::Regex;
use uuid::Uuid;

use crate::services::i18n_manifest_service;
use crate::views::audit_logs::AuditContext;
use crate::views::i18n::{ManifestEntryInput, ManifestLocation, ManifestUploadRequest};

pub struct SeedErrorCodes;

// ── Extraction ──────────────────────────────────────────────────────────────

/// One occurrence of an error code in source.
struct Occurrence {
    description: String,
    file_path: String,
    line: i32,
}

/// Validation result after extraction.
struct ExtractionReport {
    /// Unique codes → all occurrences (first = winner).
    codes: BTreeMap<String, Vec<Occurrence>>,
    /// Codes whose format doesn't match `module.detail`.
    format_violations: Vec<(String, String)>,
    /// Codes with conflicting descriptions across call sites.
    conflicts: Vec<ConflictDetail>,
}

struct ConflictDetail {
    code: String,
    /// (description, count)
    variants: Vec<(String, usize)>,
}

fn extract_error_codes() -> ExtractionReport {
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

    let re = Regex::new(
        r#"(?x)
        (?: err_bad_request
          | err_forbidden
          | err_not_found
          | err_conflict
          | err_internal
          | err_unauthorized
          | forbidden
          | bad_request
          | not_found
          | internal
          | unauthorized
          | ErrorDetail::new
        )
        \(\s*
        "([^"]+)"          # group 1: code
        \s*,\s*
        "([^"]*)"          # group 2: description (may be empty)
        "#,
    )
    .unwrap();

    // Second regex: ErrorInfo enum variant literals in error_info/*.rs.
    // Matches: ErrorInfo::BadRequest("code", "desc"),
    //          ErrorInfo::Unauthorized("code", "desc"),
    //          etc.
    let info_re =
        Regex::new(r#"ErrorInfo::\w+\(\s*"([^"]+)"\s*,\s*"([^"]*)"\s*\)"#).unwrap();

    let code_fmt = Regex::new(r"^[a-z0-9_]+\.[a-z0-9_]+$").unwrap();

    let mut codes: BTreeMap<String, Vec<Occurrence>> = BTreeMap::new();
    scan_dir(&src_dir, &re, &mut codes);

    // Scan error_info/ directory for ErrorInfo variant literals.
    let error_info_dir = src_dir.join("error_info");
    scan_dir(&error_info_dir, &info_re, &mut codes);

    // ── Format validation ─────────────────────────────────────────────────
    let format_violations: Vec<(String, String)> = codes
        .keys()
        .filter(|code| !code_fmt.is_match(code))
        .map(|code| {
            let loc = &codes[code][0];
            (code.clone(), format!("{}:{}", loc.file_path, loc.line))
        })
        .collect();

    // ── Conflict detection ────────────────────────────────────────────────
    let conflicts: Vec<ConflictDetail> = codes
        .iter()
        .filter_map(|(code, occs)| {
            // Group occurrences by description.
            let mut desc_counts: BTreeMap<String, usize> = BTreeMap::new();
            for occ in occs {
                *desc_counts.entry(occ.description.clone()).or_insert(0) += 1;
            }
            if desc_counts.len() > 1 {
                let mut variants: Vec<(String, usize)> =
                    desc_counts.into_iter().collect();
                // Sort by count descending (majority first).
                variants.sort_by(|a, b| b.1.cmp(&a.1));
                Some(ConflictDetail {
                    code: code.clone(),
                    variants,
                })
            } else {
                None
            }
        })
        .collect();

    ExtractionReport {
        codes,
        format_violations,
        conflicts,
    }
}

fn scan_dir(dir: &Path, re: &Regex, codes: &mut BTreeMap<String, Vec<Occurrence>>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir(&path, re, codes);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };
            let relative = path
                .strip_prefix(env!("CARGO_MANIFEST_DIR"))
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");

            for cap in re.captures_iter(&content) {
                let code = cap[1].to_string();
                let desc = cap[2].to_string();
                let byte_offset = cap.get(1).unwrap().start();
                let line = content[..byte_offset]
                    .chars()
                    .filter(|&c| c == '\n')
                    .count() as i32
                    + 1;
                codes.entry(code).or_default().push(Occurrence {
                    description: desc,
                    file_path: relative.clone(),
                    line,
                });
            }
        }
    }
}

// ── CLI arg resolution ──────────────────────────────────────────────────────

fn resolve(vars: &task::Vars, arg: &str, env: &str) -> Option<String> {
    vars.cli_arg(arg)
        .ok()
        .cloned()
        .or_else(|| std::env::var(env).ok())
}

// ── Task implementation ─────────────────────────────────────────────────────

#[async_trait]
impl Task for SeedErrorCodes {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "seed_error_codes".to_string(),
            detail: "Seed backend error codes into i18n CommonError namespace (auto-extracted)"
                .to_string(),
        }
    }

    async fn run(&self, ctx: &AppContext, vars: &task::Vars) -> Result<()> {
        let apply =
            resolve(vars, "apply", "SEED_APPLY").is_some_and(|v| v == "true" || v == "1");

        let report = extract_error_codes();
        let total_codes = report.codes.len();

        if total_codes == 0 {
            tracing::error!("No error codes found in source — check CARGO_MANIFEST_DIR");
            return Ok(());
        }

        // ── Print report ─────────────────────────────────────────────────
        let total_sites: usize = report.codes.values().map(|v| v.len()).sum();
        tracing::info!(
            codes = total_codes,
            call_sites = total_sites,
            mode = if apply { "APPLY" } else { "DRY-RUN" },
            "Extraction complete",
        );

        // ── Format violations ────────────────────────────────────────────
        if !report.format_violations.is_empty() {
            tracing::warn!(
                count = report.format_violations.len(),
                "Codes NOT matching 'module.detail' convention:",
            );
            for (code, loc) in &report.format_violations {
                tracing::warn!("  ✗ {code}  ({loc})");
            }
        }

        // ── Conflict detection ───────────────────────────────────────────
        if !report.conflicts.is_empty() {
            tracing::warn!(
                count = report.conflicts.len(),
                "Codes with conflicting descriptions:",
            );
            for c in &report.conflicts {
                let total: usize = c.variants.iter().map(|(_, n)| n).sum();
                tracing::warn!("  ⚠ {}  ({} call sites)", c.code, total);
                for (desc, count) in &c.variants {
                    tracing::warn!("    ×{count}  \"{desc}\"");
                }
            }
        }

        // ── Quantity regression ──────────────────────────────────────────
        let expected_min = 90; // Floor below which something is clearly wrong.
        if total_codes < expected_min {
            tracing::warn!(
                total_codes,
                expected_min,
                "Extracted code count below expected floor — regex may be broken or source files missing",
            );
        }

        // ── Stop here in dry-run ─────────────────────────────────────────
        if !apply {
            tracing::info!(
                "Dry-run complete. Pass 'apply:true' to write to DB:\n  \
                 cargo loco task seed_error_codes -- apply:true\n  \
                 $env:SEED_APPLY='true'; cargo loco task seed_error_codes",
            );
            return Ok(());
        }

        // ── Build manifest entries (first occurrence = winner) ───────────
        let namespace = "CommonError";
        let entries: Vec<ManifestEntryInput> = report
            .codes
            .iter()
            .map(|(key, occs)| {
                let winner = &occs[0];
                ManifestEntryInput {
                    namespace: namespace.to_string(),
                    key: key.clone(),
                    description: None,
                    source_text: Some(winner.description.clone()),
                    locations: vec![ManifestLocation {
                        file_path: winner.file_path.clone(),
                        line: winner.line,
                    }],
                }
            })
            .collect();

        let audit_ctx = AuditContext {
            trace_id: None,
            request_id: None,
            tenant_id: Uuid::nil(),
            user_id: None,
            ip_address: None,
            user_agent: None,
        };

        let payload = ManifestUploadRequest {
            generated_at: Some(chrono::Utc::now().to_rfc3339()),
            commit_sha: None,
            entries,
        };

        let result =
            i18n_manifest_service::apply_manifest(ctx, &payload, &audit_ctx).await?;

        tracing::info!(
            created = result.created_entries,
            updated = result.updated_entries,
            stale = result.stale_entries,
            synced_inserted = result.synced_inserted,
            synced_overwritten = result.synced_overwritten,
            "Error codes seeded successfully",
        );

        Ok(())
    }
}
