pub const MIME_BLACKLIST: &[&str] = &[
    "application/x-msdownload",
    "application/x-msi",
    "application/x-sh",
    "application/x-elf",
];

#[must_use]
pub fn detect_mime(bytes: &[u8]) -> &'static str {
    tree_magic_mini::from_u8(bytes)
}

/// Returns true if the given MIME type is blocked from upload.
///
/// Wave 2a R2: pulled out of `file_service::small_upload` as a pure
/// function so the blacklist enforcement logic can be unit-tested
/// independently of `detect_mime` (whose detection coverage on synthetic
/// payloads is limited under `tree_magic_mini` v3 — see `tests/requests/files.rs`).
#[must_use]
pub fn is_blacklisted(mime: &str) -> bool {
    MIME_BLACKLIST.contains(&mime)
}

#[cfg(test)]
mod tests {
    use super::{detect_mime, is_blacklisted, MIME_BLACKLIST};

    #[test]
    fn blacklist_contains_expected_entries() {
        assert!(MIME_BLACKLIST.contains(&"application/x-msdownload"));
        assert!(MIME_BLACKLIST.contains(&"application/x-msi"));
        assert!(MIME_BLACKLIST.contains(&"application/x-sh"));
        assert!(MIME_BLACKLIST.contains(&"application/x-elf"));
    }

    #[test]
    fn detect_mime_returns_non_empty_value_for_text() {
        assert!(!detect_mime(b"hello world\n").is_empty());
    }

    #[test]
    fn is_blacklisted_rejects_each_blacklisted_mime() {
        // Wave 2a R2: enforcement-branch coverage. If a MIME from the
        // blacklist reaches the service, it MUST be rejected. This guards
        // against accidental edits to the constant list or the contains()
        // check.
        for mime in MIME_BLACKLIST {
            assert!(
                is_blacklisted(mime),
                "blacklisted MIME `{mime}` must be rejected"
            );
        }
    }

    #[test]
    fn is_blacklisted_allows_safe_mimes() {
        for mime in [
            "text/plain",
            "image/png",
            "image/jpeg",
            "application/pdf",
            "application/json",
            "application/octet-stream",
        ] {
            assert!(
                !is_blacklisted(mime),
                "safe MIME `{mime}` must not be rejected"
            );
        }
    }
}
