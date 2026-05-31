use knota_fold::app::App;
use knota_fold::models::{
    dict_items as dict_items_model, dict_types as dict_types_model,
};
use knota_fold::services::dict_service;
use knota_fold::views::dicts::{
    CreateDictItemRequest, CreateDictTypeRequest, UpdateDictItemRequest,
    UpdateDictTypeRequest,
};
use loco_rs::prelude::model::query::PaginationQuery;
use loco_rs::testing::prelude::*;
use serial_test::serial;
use uuid::Uuid;

use knota_fold::views::audit_logs::AuditContext;

const TENANT_ID: &str = "00000000-0000-0000-0000-000000000001";
const OTHER_TENANT_ID: &str = "00000000-0000-0000-0000-000000000002";
const USER_ID: &str = "11111111-1111-1111-1111-111111111111";

const SYSTEM_GENDER_TYPE_ID: &str = "dddddddd-dddd-dddd-dddd-ddddddddd001";
const SYSTEM_STATUS_TYPE_ID: &str = "dddddddd-dddd-dddd-dddd-ddddddddd002";
const SYSTEM_MALE_ITEM_ID: &str = "eeeeeeee-eeee-eeee-eeee-eeeeeeeee001";
const SYSTEM_FEMALE_ITEM_ID: &str = "eeeeeeee-eeee-eeee-eeee-eeeeeeeee002";

fn test_audit_ctx() -> AuditContext {
    AuditContext {
        trace_id: Some("test-trace-id".to_string()),
        request_id: Some("test-request-id".to_string()),
        tenant_id: Uuid::parse_str(TENANT_ID).unwrap(),
        user_id: Some(Uuid::parse_str(USER_ID).unwrap()),
        ip_address: Some("127.0.0.1".to_string()),
        user_agent: Some("test-agent".to_string()),
    }
}

fn uuid(value: &str) -> Uuid {
    Uuid::parse_str(value).unwrap()
}

fn pagination() -> PaginationQuery {
    serde_json::from_value(serde_json::json!({
        "page_size": "10",
        "page": "1"
    }))
    .unwrap()
}

#[tokio::test]
#[serial]
async fn super_admin_can_list_system_types() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let result = dict_service::list_dict_types(db, None, &pagination())
        .await
        .expect("Failed to list system types");

    assert_eq!(result.total_items, 2);
    assert_eq!(result.items.len(), 2);
    assert!(result.items.iter().all(|item| item.scope == "system"));
    assert!(result.items.iter().any(|item| item.code == "sys.gender"));
    assert!(result.items.iter().any(|item| item.code == "sys.status"));
}

#[tokio::test]
#[serial]
async fn tenant_sees_effective_types() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let result = dict_service::list_dict_types(db, Some(uuid(TENANT_ID)), &pagination())
        .await
        .expect("Failed to list effective types");

    assert_eq!(result.total_items, 2);
    assert_eq!(result.items.len(), 2);
    assert!(result.items.iter().all(|item| item.scope == "system"));
}

#[tokio::test]
#[serial]
async fn super_admin_can_create_system_type() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let created = dict_service::create_dict_type(
        db,
        None,
        uuid(USER_ID),
        &CreateDictTypeRequest {
            code: "sys.priority".to_string(),
            name: "优先级".to_string(),
            description: Some("System priority".to_string()),
        },
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to create system type");

    assert_eq!(created.code, "sys.priority");
    assert_eq!(created.tenant_id, None);
    assert_eq!(created.source_type_id, None);
    assert_eq!(created.status, "active");
}

#[tokio::test]
#[serial]
async fn tenant_can_create_own_type() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;
    let tenant_id = uuid(TENANT_ID);

    let created = dict_service::create_dict_type(
        db,
        Some(tenant_id),
        uuid(USER_ID),
        &CreateDictTypeRequest {
            code: "tenant.priority".to_string(),
            name: "租户优先级".to_string(),
            description: Some("Tenant only type".to_string()),
        },
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to create tenant type");

    assert_eq!(created.code, "tenant.priority");
    assert_eq!(created.tenant_id, Some(tenant_id));
    assert_eq!(created.source_type_id, None);
}

#[tokio::test]
#[serial]
async fn tenant_cannot_create_code_conflicting_with_system() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let err = dict_service::create_dict_type(
        db,
        Some(uuid(TENANT_ID)),
        uuid(USER_ID),
        &CreateDictTypeRequest {
            code: "sys.gender".to_string(),
            name: "冲突编码".to_string(),
            description: None,
        },
        &test_audit_ctx(),
    )
    .await
    .expect_err("Tenant should not create a code conflicting with system type");

    assert!(
        format!("{err:?}").contains("系统字典冲突"),
        "Error should mention system dict conflict, got: {err:?}"
    );
}

#[tokio::test]
#[serial]
async fn super_admin_can_update_system_type() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let updated = dict_service::update_dict_type(
        db,
        uuid(SYSTEM_GENDER_TYPE_ID),
        None,
        uuid(USER_ID),
        &UpdateDictTypeRequest {
            name: Some("性别-更新".to_string()),
            description: Some(Some("Updated by super admin".to_string())),
            version: 1,
        },
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to update system type");

    assert_eq!(updated.tenant_id, None);
    assert_eq!(updated.name, "性别-更新");
    assert_eq!(
        updated.description.as_deref(),
        Some("Updated by super admin")
    );
    assert_eq!(updated.version, 2);
}

#[tokio::test]
#[serial]
async fn tenant_update_system_type_creates_override() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;
    let tenant_id = uuid(TENANT_ID);
    let system_type_id = uuid(SYSTEM_GENDER_TYPE_ID);

    let updated = dict_service::update_dict_type(
        db,
        system_type_id,
        Some(tenant_id),
        uuid(USER_ID),
        &UpdateDictTypeRequest {
            name: Some("租户性别".to_string()),
            description: Some(Some("Tenant override".to_string())),
            version: 1,
        },
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to create override for system type");

    assert_eq!(updated.tenant_id, Some(tenant_id));
    assert_eq!(updated.source_type_id, Some(system_type_id));
    assert_eq!(updated.code, "sys.gender");
    assert_eq!(updated.name, "租户性别");
    assert_eq!(updated.version, 1);
}

#[tokio::test]
#[serial]
async fn update_dict_type_version_conflict() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let err = dict_service::update_dict_type(
        db,
        uuid(SYSTEM_GENDER_TYPE_ID),
        None,
        uuid(USER_ID),
        &UpdateDictTypeRequest {
            name: Some("冲突更新".to_string()),
            description: None,
            version: 999,
        },
        &test_audit_ctx(),
    )
    .await
    .expect_err("Expected version conflict");

    assert!(
        format!("{err:?}").contains("Version conflict"),
        "Error should mention version conflict, got: {err:?}"
    );
}

#[tokio::test]
#[serial]
async fn super_admin_can_toggle_system_type() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let toggled = dict_service::toggle_dict_type_status(
        db,
        uuid(SYSTEM_GENDER_TYPE_ID),
        None,
        uuid(USER_ID),
        1,
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to toggle system type");

    assert_eq!(toggled.tenant_id, None);
    assert_eq!(toggled.status, "disabled");
    assert_eq!(toggled.version, 2);
}

#[tokio::test]
#[serial]
async fn tenant_toggle_system_type_creates_disabled_override() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;
    let tenant_id = uuid(TENANT_ID);
    let system_type_id = uuid(SYSTEM_GENDER_TYPE_ID);

    let toggled = dict_service::toggle_dict_type_status(
        db,
        system_type_id,
        Some(tenant_id),
        uuid(USER_ID),
        1,
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to toggle system type for tenant");

    assert_eq!(toggled.tenant_id, Some(tenant_id));
    assert_eq!(toggled.source_type_id, Some(system_type_id));
    assert_eq!(toggled.status, "disabled");
    assert_eq!(toggled.version, 1);
}

#[tokio::test]
#[serial]
async fn tenant_can_reset_type_override() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;
    let tenant_id = uuid(TENANT_ID);
    let system_type_id = uuid(SYSTEM_GENDER_TYPE_ID);

    let override_row = dict_service::update_dict_type(
        db,
        system_type_id,
        Some(tenant_id),
        uuid(USER_ID),
        &UpdateDictTypeRequest {
            name: Some("租户覆盖".to_string()),
            description: None,
            version: 1,
        },
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to create type override");

    dict_service::reset_dict_type_override(
        db,
        override_row.id,
        tenant_id,
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to reset type override");

    let result = dict_service::list_dict_types(db, Some(tenant_id), &pagination())
        .await
        .expect("Failed to list types after reset");
    let gender = result
        .items
        .iter()
        .find(|item| item.code == "sys.gender")
        .expect("sys.gender should still be visible after reset");

    assert_eq!(gender.scope, "system");
    assert!(!gender.is_override);
    assert!(dict_types_model::Model::find_override_by_tenant_and_source(
        db,
        tenant_id,
        system_type_id
    )
    .await
    .is_err());
}

#[tokio::test]
#[serial]
async fn tenant_can_list_items_by_type_code() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let items = dict_service::list_dict_items(db, Some(uuid(TENANT_ID)), "sys.gender")
        .await
        .expect("Failed to list tenant-visible items");

    assert_eq!(items.len(), 2);
    assert!(items.iter().any(|item| item.code == "male"));
    assert!(items.iter().any(|item| item.code == "female"));
    assert!(items.iter().all(|item| item.scope == "system"));
}

#[tokio::test]
#[serial]
async fn super_admin_can_list_system_items() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let items = dict_service::list_dict_items(db, None, "sys.gender")
        .await
        .expect("Failed to list system items");

    assert_eq!(items.len(), 2);
    assert!(items.iter().all(|item| item.scope == "system"));
    assert!(items.iter().any(|item| item.id == SYSTEM_MALE_ITEM_ID));
    assert!(items.iter().any(|item| item.id == SYSTEM_FEMALE_ITEM_ID));
}

#[tokio::test]
#[serial]
async fn tenant_can_create_item_under_system_type() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;
    let tenant_id = uuid(TENANT_ID);
    let type_id = uuid(SYSTEM_GENDER_TYPE_ID);

    let created = dict_service::create_dict_item(
        db,
        Some(tenant_id),
        uuid(USER_ID),
        &CreateDictItemRequest {
            dict_type_id: type_id,
            code: "other".to_string(),
            name: "其他".to_string(),
            value: "3".to_string(),
            parent_id: None,
            sort_order: Some(3),
            description: Some("Tenant item".to_string()),
        },
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to create tenant item under system type");

    assert_eq!(created.tenant_id, Some(tenant_id));
    assert_eq!(created.dict_type_id, type_id);
    assert_eq!(created.source_item_id, None);
    assert_eq!(created.code, "other");
}

#[tokio::test]
#[serial]
async fn tenant_update_system_item_creates_override() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;
    let tenant_id = uuid(TENANT_ID);
    let item_id = uuid(SYSTEM_MALE_ITEM_ID);

    let updated = dict_service::update_dict_item(
        db,
        item_id,
        Some(tenant_id),
        uuid(USER_ID),
        &UpdateDictItemRequest {
            name: Some("男性-租户".to_string()),
            parent_id: None,
            sort_order: Some(11),
            description: Some(Some("Tenant item override".to_string())),
            version: 1,
        },
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to create item override");

    assert_eq!(updated.tenant_id, Some(tenant_id));
    assert_eq!(updated.source_item_id, Some(item_id));
    assert_eq!(updated.name, "男性-租户");
    assert_eq!(updated.sort_order, 11);
    assert_eq!(updated.version, 1);
}

#[tokio::test]
#[serial]
async fn tenant_toggle_system_item_creates_disabled_override() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;
    let tenant_id = uuid(TENANT_ID);
    let item_id = uuid(SYSTEM_MALE_ITEM_ID);

    let toggled = dict_service::toggle_dict_item_status(
        db,
        item_id,
        Some(tenant_id),
        uuid(USER_ID),
        1,
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to create disabled item override");

    assert_eq!(toggled.tenant_id, Some(tenant_id));
    assert_eq!(toggled.source_item_id, Some(item_id));
    assert_eq!(toggled.status, "disabled");
    assert_eq!(toggled.version, 1);
}

#[tokio::test]
#[serial]
async fn tenant_can_reset_item_override() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;
    let tenant_id = uuid(TENANT_ID);
    let item_id = uuid(SYSTEM_MALE_ITEM_ID);

    let override_row = dict_service::update_dict_item(
        db,
        item_id,
        Some(tenant_id),
        uuid(USER_ID),
        &UpdateDictItemRequest {
            name: Some("男性-覆盖".to_string()),
            parent_id: None,
            sort_order: Some(7),
            description: None,
            version: 1,
        },
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to create item override");

    dict_service::reset_dict_item_override(
        db,
        override_row.id,
        tenant_id,
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to reset item override");

    let items = dict_service::list_dict_items(db, Some(tenant_id), "sys.gender")
        .await
        .expect("Failed to list items after reset");
    let male = items
        .iter()
        .find(|item| item.code == "male")
        .expect("male item should remain visible after reset");

    assert_eq!(male.scope, "system");
    assert!(!male.is_override);
    assert!(dict_items_model::Model::find_override_by_tenant_and_source(
        db, tenant_id, item_id
    )
    .await
    .is_err());
}

#[tokio::test]
#[serial]
async fn tenant_isolation_only_sees_own_data() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;
    let tenant_a = uuid(TENANT_ID);
    let tenant_b = uuid(OTHER_TENANT_ID);

    dict_service::create_dict_type(
        db,
        Some(tenant_a),
        uuid(USER_ID),
        &CreateDictTypeRequest {
            code: "tenant.shared".to_string(),
            name: "Tenant A only".to_string(),
            description: None,
        },
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to create tenant A type");

    let tenant_a_result =
        dict_service::list_dict_types(db, Some(tenant_a), &pagination())
            .await
            .expect("Failed to list tenant A types");
    let tenant_b_result =
        dict_service::list_dict_types(db, Some(tenant_b), &pagination())
            .await
            .expect("Failed to list tenant B types");

    assert_eq!(tenant_a_result.total_items, 3);
    assert!(tenant_a_result
        .items
        .iter()
        .any(|item| item.code == "tenant.shared"));
    assert_eq!(tenant_b_result.total_items, 2);
    assert!(tenant_b_result
        .items
        .iter()
        .all(|item| item.code != "tenant.shared"));
    assert!(tenant_b_result
        .items
        .iter()
        .any(|item| item.code == "sys.gender"));
    assert!(tenant_b_result
        .items
        .iter()
        .any(|item| item.code == "sys.status"));
}

#[tokio::test]
#[serial]
async fn tenant_can_have_same_code_as_other_tenants_tenant_only() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;
    let tenant_a = uuid(TENANT_ID);
    let tenant_b = uuid(OTHER_TENANT_ID);

    let created_a = dict_service::create_dict_type(
        db,
        Some(tenant_a),
        uuid(USER_ID),
        &CreateDictTypeRequest {
            code: "tenant.dup".to_string(),
            name: "Tenant A Dup".to_string(),
            description: None,
        },
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to create tenant A duplicate code type");

    let created_b = dict_service::create_dict_type(
        db,
        Some(tenant_b),
        uuid(USER_ID),
        &CreateDictTypeRequest {
            code: "tenant.dup".to_string(),
            name: "Tenant B Dup".to_string(),
            description: None,
        },
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to create tenant B duplicate code type");

    let tenant_b_result =
        dict_service::list_dict_types(db, Some(tenant_b), &pagination())
            .await
            .expect("Failed to list tenant B types");

    assert_eq!(created_a.tenant_id, Some(tenant_a));
    assert_eq!(created_b.tenant_id, Some(tenant_b));
    assert_ne!(created_a.id, created_b.id);
    assert!(tenant_b_result.items.iter().any(|item| {
        item.code == "tenant.dup"
            && item.name == "Tenant B Dup"
            && item.scope == "tenantOnly"
    }));
}

#[tokio::test]
#[serial]
async fn super_admin_can_list_system_item_tree() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let tree = dict_service::get_dict_item_tree(db, None, "sys.gender")
        .await
        .expect("Failed to get system item tree");

    assert_eq!(tree.len(), 2);
    assert!(tree.iter().all(|node| node.children.is_empty()));
    assert!(tree.iter().all(|node| node.scope == "system"));
}

#[tokio::test]
#[serial]
async fn tenant_can_get_effective_item_tree() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let tree = dict_service::get_dict_item_tree(db, Some(uuid(TENANT_ID)), "sys.gender")
        .await
        .expect("Failed to get tenant item tree");

    assert_eq!(tree.len(), 2);
    assert!(tree.iter().all(|node| node.children.is_empty()));
}

#[tokio::test]
#[serial]
async fn tenant_can_create_item_under_status_system_type() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let created = dict_service::create_dict_item(
        db,
        Some(uuid(TENANT_ID)),
        uuid(USER_ID),
        &CreateDictItemRequest {
            dict_type_id: uuid(SYSTEM_STATUS_TYPE_ID),
            code: "archived".to_string(),
            name: "归档".to_string(),
            value: "2".to_string(),
            parent_id: None,
            sort_order: Some(3),
            description: None,
        },
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to create tenant status item");

    assert_eq!(created.tenant_id, Some(uuid(TENANT_ID)));
    assert_eq!(created.dict_type_id, uuid(SYSTEM_STATUS_TYPE_ID));
    assert_eq!(created.code, "archived");
}
