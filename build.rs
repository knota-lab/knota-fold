use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

const ALLOW_MARKER: &str = "raw-sql-ok:";
const ALLOW_FILE_MARKER: &str = "raw-sql-ok-file:";

const RAW_SQL_PATTERNS: &[&str] = &[
    "execute_unprepared(",
    "Statement::from_sql_and_values(",
    "Statement::from_string(",
    "sqlx::query(",
    "sqlx::query_as(",
    "sqlx::query_scalar(",
    "raw_sql!(",
];

const EXCLUDED_DIRS: &[&str] = &[
    "src/app_logs", // dedicated sqlx-backed SQLite log store
];

const EXCLUDED_FILES: &[&str] = &[
    "src/initializers/sqlite_wal.rs", // SQLite PRAGMA setup
];

#[derive(Clone, Copy)]
struct LineRule {
    id: &'static str,
    severity: Severity,
    marker: &'static str,
    include_prefixes: &'static [&'static str],
    patterns: &'static [&'static str],
    message: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Severity {
    Error,
    Warning,
}

const LINE_RULES: &[LineRule] = &[
    LineRule {
        id: "no-error-string",
        severity: Severity::Error,
        marker: "no-error-string-ok:",
        include_prefixes: &[
            "src/controllers/",
            "src/services/",
            "src/extractors/",
            "src/middleware/",
        ],
        patterns: &["Error::string("],
        message: "禁止使用 Error::string()（会变成 500），请使用 views::errors 辅助函数或 AppError",
    },
    LineRule {
        id: "no-domain-to-err",
        severity: Severity::Warning,
        marker: "no-domain-to-err-ok:",
        include_prefixes: &["src/controllers/", "src/modules/"],
        patterns: &[".to_err()"],
        message: "Domain 错误的 to_err() 会丢失结构化信息变成 500，请使用 to_response() 或转换为 AppError",
    },
];

#[derive(Clone, Copy)]
struct WindowRule {
    id: &'static str,
    severity: Severity,
    marker: &'static str,
    include_prefixes: &'static [&'static str],
    anchor: &'static str,
    required_parts: &'static [&'static str],
    window_lines: usize,
    message: &'static str,
}

const WINDOW_RULES: &[WindowRule] = &[
    WindowRule {
        id: "no-map-err-to-model-error-any",
        severity: Severity::Error,
        marker: "no-map-err-to-model-error-any-ok:",
        include_prefixes: &["src/models/", "src/modules/", "src/services/"],
        anchor: ".map_err(|",
        required_parts: &["ModelError::Any", ".into()"],
        window_lines: 6,
        message: "禁止在 models/modules/services 中使用 map_err(|e| ModelError::Any(e.into()))；SeaORM DbErr 通常可直接用 ?，其他错误请显式转换",
    },
    WindowRule {
        id: "no-uuid-parse-to-error-any",
        severity: Severity::Warning,
        marker: "no-uuid-parse-to-error-any-ok:",
        include_prefixes: &["src/controllers/"],
        anchor: ".parse::<Uuid>()",
        required_parts: &[".map_err(|", "Error::Any", ".into()"],
        window_lines: 6,
        message: "UUID 解析失败不应使用 Error::Any（会变成 500），请使用 Error::BadRequest 或 AppError::BadRequest",
    },
];

fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=KNOTA_QUALITY_WARNINGS");

    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .expect("CARGO_MANIFEST_DIR should be set by Cargo"),
    );
    let src = manifest_dir.join("src");
    let mut report = QualityReport::default();

    visit_rs_files(&src, &mut |path| {
        report.extend(check_file(&manifest_dir, path));
    });

    if std::env::var("KNOTA_QUALITY_WARNINGS").as_deref() == Ok("1") {
        for warning in &report.warnings {
            println!("cargo:warning={warning}");
        }
    } else if !report.warnings.is_empty() {
        println!(
            "cargo:warning={} code quality warning(s) suppressed; set KNOTA_QUALITY_WARNINGS=1 for details",
            report.warnings.len()
        );
    }

    assert!(
        report.errors.is_empty(),
        "Code quality guard failed.\n\n{}",
        report.errors.join("\n")
    );
}

#[derive(Default)]
struct QualityReport {
    errors: Vec<String>,
    warnings: Vec<String>,
}

impl QualityReport {
    fn push(&mut self, severity: Severity, violation: String) {
        match severity {
            Severity::Error => self.errors.push(violation),
            Severity::Warning => self.warnings.push(violation),
        }
    }

    fn extend(&mut self, other: Self) {
        self.errors.extend(other.errors);
        self.warnings.extend(other.warnings);
    }
}

fn visit_rs_files(dir: &Path, f: &mut impl FnMut(&Path)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            visit_rs_files(&path, f);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            f(&path);
        }
    }
}

fn check_file(manifest_dir: &Path, path: &Path) -> QualityReport {
    let rel = path
        .strip_prefix(manifest_dir)
        .expect("checked path should be inside manifest dir");
    let rel_slash = rel.to_string_lossy().replace('\\', "/");

    if EXCLUDED_FILES.contains(&rel_slash.as_str())
        || EXCLUDED_DIRS
            .iter()
            .any(|dir| rel_slash == *dir || rel_slash.starts_with(&format!("{dir}/")))
    {
        return QualityReport::default();
    }

    let Ok(content) = fs::read_to_string(path) else {
        return QualityReport::default();
    };
    let lines: Vec<&str> = content.lines().collect();
    if lines
        .iter()
        .take(12)
        .any(|line| line.contains(ALLOW_FILE_MARKER))
    {
        return QualityReport::default();
    }

    let mut report = QualityReport::default();
    for violation in check_raw_sql(&rel_slash, &lines) {
        report.push(Severity::Error, violation);
    }
    report.extend(check_line_rules(&rel_slash, &lines));
    report.extend(check_window_rules(&rel_slash, &lines));

    report
}

fn check_raw_sql(rel_slash: &str, lines: &[&str]) -> Vec<String> {
    let mut violations = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if !RAW_SQL_PATTERNS
            .iter()
            .any(|pattern| line.contains(pattern))
        {
            continue;
        }
        if has_allow_nearby(lines, index) {
            continue;
        }
        violations.push(format!(
            "{}:{}: raw SQL requires a nearby `{ALLOW_MARKER}` comment",
            rel_slash,
            index + 1
        ));
    }
    violations
}

fn check_line_rules(rel_slash: &str, lines: &[&str]) -> QualityReport {
    let mut report = QualityReport::default();
    for rule in LINE_RULES {
        if !matches_prefix(rel_slash, rule.include_prefixes) {
            continue;
        }
        for (index, line) in lines.iter().enumerate() {
            if !rule.patterns.iter().any(|pattern| line.contains(pattern)) {
                continue;
            }
            if has_marker_nearby(lines, index, rule.marker) {
                continue;
            }
            report.push(
                rule.severity,
                format_rule_violation(
                    rel_slash,
                    index,
                    rule.id,
                    rule.marker,
                    rule.message,
                ),
            );
        }
    }
    report
}

fn check_window_rules(rel_slash: &str, lines: &[&str]) -> QualityReport {
    let mut report = QualityReport::default();
    for rule in WINDOW_RULES {
        if !matches_prefix(rel_slash, rule.include_prefixes) {
            continue;
        }
        for (index, line) in lines.iter().enumerate() {
            if !line.contains(rule.anchor) {
                continue;
            }
            let end = lines.len().min(index + rule.window_lines);
            let joined = lines[index..end].join(" ");
            if !rule.required_parts.iter().all(|part| joined.contains(part)) {
                continue;
            }
            if has_marker_nearby(lines, index, rule.marker) {
                continue;
            }
            report.push(
                rule.severity,
                format_rule_violation(
                    rel_slash,
                    index,
                    rule.id,
                    rule.marker,
                    rule.message,
                ),
            );
        }
    }
    report
}

fn matches_prefix(rel_slash: &str, prefixes: &[&str]) -> bool {
    prefixes
        .iter()
        .any(|prefix| rel_slash == *prefix || rel_slash.starts_with(prefix))
}

fn format_rule_violation(
    rel_slash: &str,
    index: usize,
    id: &str,
    marker: &str,
    message: &str,
) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "{rel_slash}:{}: {id}: {message}\n  Add `{marker}` with a reason on the same line or one of the two previous lines.",
        index + 1
    );
    out
}

fn has_allow_nearby(lines: &[&str], index: usize) -> bool {
    has_marker_nearby(lines, index, ALLOW_MARKER)
}

fn has_marker_nearby(lines: &[&str], index: usize, marker: &str) -> bool {
    let start = index.saturating_sub(2);
    lines[start..=index]
        .iter()
        .any(|line| line.contains(marker))
}
