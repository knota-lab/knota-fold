use chrono::{Datelike, Utc};
use uuid::Uuid;

use crate::views::errors::err_bad_request;

const B3_PREFIX: &str = "b3:";
const B3_HEX_LEN: usize = 64;

#[must_use]
pub fn format_b3_hash(hash: &blake3::Hash) -> String {
    format!("{B3_PREFIX}{}", hash.to_hex())
}

pub fn build_storage_key(
    hash: &str,
    tenant_id: Uuid,
    env: &str,
    file_id: Uuid,
) -> loco_rs::Result<String> {
    let stripped = validate_b3_hash(hash)?;
    let env = env.trim();

    if env.is_empty() {
        return Err(err_bad_request("file.env_empty", "environment 不能为空"));
    }

    let now = Utc::now();

    Ok(format!(
        "{}/{}/{}/{:04}/{:02}/{}",
        &stripped[..2],
        tenant_id,
        env,
        now.year(),
        now.month(),
        file_id,
    ))
}

pub fn validate_b3_hash(hash: &str) -> loco_rs::Result<&str> {
    let stripped = hash.strip_prefix(B3_PREFIX).ok_or_else(|| {
        err_bad_request("file.hash_prefix_invalid", "contentHash 必须以 b3: 开头")
    })?;

    if stripped.len() != B3_HEX_LEN {
        return Err(err_bad_request(
            "file.hash_length_invalid",
            "contentHash 必须包含 64 个小写十六进制字符",
        ));
    }

    if !stripped
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(err_bad_request(
            "file.hash_hex_invalid",
            "contentHash 只能包含小写十六进制字符",
        ));
    }

    Ok(stripped)
}

#[cfg(test)]
mod tests {
    use super::{build_storage_key, format_b3_hash};
    use uuid::Uuid;

    #[test]
    fn format_b3_hash_hello_world_nl() {
        let hash = blake3::hash(b"hello world\n");
        assert_eq!(
            format_b3_hash(&hash),
            // Wave 2a: verified against the local blake3 crate for the literal
            // byte string b"hello world\n"; approved-plan constant appears transcribed incorrectly.
            "b3:dc5a4edb8240b018124052c330270696f96771a63b45250a5c17d3000e823355"
        );
    }

    #[test]
    fn build_storage_key_uses_hash_prefix_without_b3_prefix() {
        let tenant_id = Uuid::parse_str("11111111-1111-1111-1111-111111111111")
            .expect("static UUID literal is valid");
        let file_id = Uuid::parse_str("22222222-2222-2222-2222-222222222222")
            .expect("static UUID literal is valid");

        let key = build_storage_key(
            "b3:ab5a4eda8240b018124052c330270696f96771a63b45250a5c17d3000e823555",
            tenant_id,
            "development",
            file_id,
        )
        .expect("valid hash should build storage key");

        assert!(key.starts_with("ab/11111111-1111-1111-1111-111111111111/development/"));
        assert!(key.ends_with("/22222222-2222-2222-2222-222222222222"));
    }
}
