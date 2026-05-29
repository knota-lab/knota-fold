use loco_rs::{controller::ErrorDetail, prelude::*};
use serde::Deserialize;
use uuid::Uuid;

use crate::utils::error::{IntoAppError, IntoLocoResult};
use crate::{models::sys_configs, views::file_uploads::ProbeUploadHint};

const POLICY_KEY: &str = "files.multipart.partSizePolicy";
const MIB: u64 = 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyTier {
    pub min_size: u64,
    pub max_size: Option<u64>,
    pub part_size: Option<u64>,
    pub part_size_rule: Option<String>,
    pub parts_total_cap: u32,
    pub concurrency_hint: u32,
    pub endpoint: String,
}

pub async fn load_policy_config(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> loco_rs::Result<Vec<PolicyTier>> {
    let value = if let Some(tenant_config) =
        sys_configs::Model::find_tenant_by_key(db, POLICY_KEY, tenant_id)
            .await
            .db_err()?
    {
        tenant_config.value
    } else {
        sys_configs::Model::find_global_by_key(db, POLICY_KEY)
            .await
            .db_err()?
            .map(|config| config.value)
            .ok_or_else(|| {
                Error::CustomError(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorDetail::new(
                        "partition_policy.config_missing",
                        "missing files.multipart.partSizePolicy sys_config",
                    ),
                )
            })?
    };

    serde_json::from_str::<Vec<PolicyTier>>(&value).loco_err()
}

pub fn compute(
    file_size: u64,
    policy_config: &[PolicyTier],
) -> loco_rs::Result<ProbeUploadHint> {
    let tier = policy_config
        .iter()
        .find(|tier| {
            file_size >= tier.min_size
                && tier.max_size.is_none_or(|max_size| file_size < max_size)
        })
        .ok_or_else(|| {
            Error::CustomError(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                ErrorDetail::new(
                    "partition_policy.no_tier_matched",
                    "no part size policy tier matched file size",
                ),
            )
        })?;

    let part_size = match (tier.part_size, tier.part_size_rule.as_deref()) {
        (Some(part_size), _) => part_size,
        (None, Some("file_size")) => file_size,
        (None, Some("ceil_div_8000_rounded_up_to_mib")) => {
            let raw_bytes = file_size.div_ceil(8000);
            raw_bytes.div_ceil(MIB) * MIB
        }
        _ => {
            return Err(Error::CustomError(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                ErrorDetail::new("partition_policy.invalid_tier", "invalid part size policy tier: missing part_size or supported part_size_rule"),
            ));
        }
    };

    if part_size == 0 {
        return Err(Error::CustomError(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            ErrorDetail::new(
                "partition_policy.zero_part_size",
                "computed part_size must be greater than 0",
            ),
        ));
    }

    let parts_total_u64 = file_size.div_ceil(part_size);
    let parts_total = u32::try_from(parts_total_u64).map_err(|_| {
        crate::views::errors::err_custom(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "too_many_parts",
            "partsTotal overflow",
        )
    })?;

    if parts_total > tier.parts_total_cap {
        return Err(crate::views::errors::err_custom(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "too_many_parts",
            "partsTotal exceeds configured tier cap",
        ));
    }

    Ok(ProbeUploadHint {
        endpoint: tier.endpoint.clone(),
        part_size,
        parts_total,
        concurrency_hint: tier.concurrency_hint,
        requires_full_hash: true,
    })
}

#[cfg(test)]
mod partition_policy {
    use super::*;

    const GIB: u64 = 1024 * MIB;

    fn policy() -> Vec<PolicyTier> {
        vec![
            PolicyTier {
                min_size: 0,
                max_size: Some(5 * MIB),
                part_size: None,
                part_size_rule: Some("file_size".to_string()),
                parts_total_cap: 1,
                concurrency_hint: 1,
                endpoint: "/api/files".to_string(),
            },
            PolicyTier {
                min_size: 5 * MIB,
                max_size: Some(100 * MIB),
                part_size: Some(5 * MIB),
                part_size_rule: None,
                parts_total_cap: 20,
                concurrency_hint: 4,
                endpoint: "/api/file-uploads".to_string(),
            },
            PolicyTier {
                min_size: 100 * MIB,
                max_size: Some(GIB),
                part_size: Some(10 * MIB),
                part_size_rule: None,
                parts_total_cap: 100,
                concurrency_hint: 6,
                endpoint: "/api/file-uploads".to_string(),
            },
            PolicyTier {
                min_size: GIB,
                max_size: Some(10 * GIB),
                part_size: Some(25 * MIB),
                part_size_rule: None,
                parts_total_cap: 400,
                concurrency_hint: 6,
                endpoint: "/api/file-uploads".to_string(),
            },
            PolicyTier {
                min_size: 10 * GIB,
                max_size: None,
                part_size: Some(100 * MIB),
                part_size_rule: None,
                parts_total_cap: 1024,
                concurrency_hint: 8,
                endpoint: "/api/file-uploads".to_string(),
            },
        ]
    }

    #[test]
    fn partition_policy_tier_under_5_mib_uses_single_request() {
        let hint = compute(5 * MIB - 1, &policy()).unwrap();
        assert_eq!(hint.endpoint, "/api/files");
        assert_eq!(hint.part_size, 5 * MIB - 1);
        assert_eq!(hint.parts_total, 1);
        assert_eq!(hint.concurrency_hint, 1);
    }

    #[test]
    fn partition_policy_tier_5_mib_to_100_mib_is_half_open() {
        let hint = compute(5 * MIB, &policy()).unwrap();
        assert_eq!(hint.part_size, 5 * MIB);
        assert_eq!(hint.parts_total, 1);

        let upper = compute(100 * MIB - 1, &policy()).unwrap();
        assert_eq!(upper.part_size, 5 * MIB);
        assert_eq!(upper.parts_total, 20);
        assert_eq!(upper.concurrency_hint, 4);
    }

    #[test]
    fn partition_policy_tier_100_mib_to_1_gib_is_half_open() {
        let hint = compute(100 * MIB, &policy()).unwrap();
        assert_eq!(hint.part_size, 10 * MIB);
        assert_eq!(hint.parts_total, 10);

        let upper = compute(1000 * MIB, &policy()).unwrap();
        assert_eq!(upper.part_size, 10 * MIB);
        assert_eq!(upper.parts_total, 100);
        assert_eq!(upper.concurrency_hint, 6);
    }

    #[test]
    fn partition_policy_tier_1_gib_to_10_gib_is_half_open() {
        let hint = compute(GIB, &policy()).unwrap();
        assert_eq!(hint.part_size, 25 * MIB);

        let upper = compute(400 * 25 * MIB, &policy()).unwrap();
        assert_eq!(upper.part_size, 25 * MIB);
        assert_eq!(upper.parts_total, 400);
        assert_eq!(upper.concurrency_hint, 6);
    }

    #[test]
    fn partition_policy_tier_10_gib_and_above_covers_up_to_100_gib() {
        let hint = compute(10 * GIB, &policy()).unwrap();
        assert_eq!(hint.part_size, 100 * MIB);

        // 100 GiB is the validation cap (inclusive); 100 GiB / 100 MiB = 1024 parts exactly.
        let upper = compute(100 * GIB, &policy()).unwrap();
        assert_eq!(upper.part_size, 100 * MIB);
        assert_eq!(upper.parts_total, 1024);
        assert_eq!(upper.concurrency_hint, 8);
    }
}
