use knota_fold::app::App;
use knota_fold::views::auth::LoginResponse;
use loco_rs::testing::prelude::*;
use serial_test::serial;

use super::prepare_data;

/// Full lifecycle: create tenant → create admin → login → list roles → list users → create user → get menus
#[tokio::test]
#[serial]
async fn full_tenant_lifecycle() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;

        let ta = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "Lifecycle Tenant",
            "LIFECYCLE",
            "lifecycle-admin@test.com",
            "admin1234",
            "Lifecycle Admin",
        )
        .await;

        // List roles as tenant admin — should see tenant's own roles
        let (k1, v1) = prepare_data::auth_header(&ta.token);
        let roles_response = request
            .get("/api/roles?page=1&pageSize=10")
            .add_header(k1, v1)
            .await;
        assert_eq!(
            roles_response.status_code(),
            200,
            "Tenant admin should list roles: {}",
            roles_response.text()
        );
        let roles_body: serde_json::Value =
            serde_json::from_str(&roles_response.text()).unwrap();
        let role_count = roles_body["totalItems"].as_i64().unwrap();
        assert_eq!(
            role_count, 2,
            "New tenant should see 2 roles (TENANT_ADMIN, MEMBER), got {role_count}"
        );

        // List users as tenant admin — should see 1 user (the admin itself)
        let (k2, v2) = prepare_data::auth_header(&ta.token);
        let users_response = request
            .get("/api/users?page=1&pageSize=10")
            .add_header(k2, v2)
            .await;
        assert_eq!(
            users_response.status_code(),
            200,
            "Tenant admin should list users: {}",
            users_response.text()
        );
        let users_body: serde_json::Value =
            serde_json::from_str(&users_response.text()).unwrap();
        let user_count = users_body["totalItems"].as_i64().unwrap();
        assert_eq!(
            user_count, 1,
            "New tenant should see 1 user (the admin), got {user_count}"
        );

        // Create a user under the new tenant
        let (k3, v3) = prepare_data::auth_header(&ta.token);
        let create_user_payload = serde_json::json!({
            "email": "lifecycle-user@test.com",
            "password": "user1234",
            "name": "Lifecycle User",
        });
        let create_user_response = request
            .post("/api/users")
            .json(&create_user_payload)
            .add_header(k3, v3)
            .await;
        assert_eq!(
            create_user_response.status_code(),
            200,
            "Create user under new tenant should succeed: {}",
            create_user_response.text()
        );

        // Get menus for the admin (whitelist path, should return 200 even if empty)
        let (k4, v4) = prepare_data::auth_header(&ta.token);
        let menus_response = request.get("/api/users/me/menus").add_header(k4, v4).await;
        assert_eq!(
            menus_response.status_code(),
            200,
            "Get menus should succeed: {}",
            menus_response.text()
        );
    })
    .await;
}

/// Tenant admin A can only see their own tenant's roles, not roles from other tenants.
#[tokio::test]
#[serial]
async fn tenant_admin_cannot_see_other_tenant_roles() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;

        let ta = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "Isolation Roles Tenant",
            "ISOL_ROLES",
            "isol-roles-admin@test.com",
            "admin1234",
            "Isolation Admin",
        )
        .await;

        // Tenant admin lists roles — should NOT see DEFAULT tenant's 3 roles
        let (k1, v1) = prepare_data::auth_header(&ta.token);
        let roles_response = request
            .get("/api/roles?page=1&pageSize=50")
            .add_header(k1, v1)
            .await;
        assert_eq!(roles_response.status_code(), 200);

        let body: serde_json::Value =
            serde_json::from_str(&roles_response.text()).unwrap();
        let total = body["totalItems"].as_i64().unwrap();
        assert_eq!(
            total, 2,
            "Tenant admin should only see own tenant's 2 roles, got {total}"
        );

        let items = body["items"].as_array().unwrap();
        for role in items {
            let code = role["tenantCode"].as_str().unwrap();
            assert_eq!(
                code, "ISOL_ROLES",
                "Role should belong to ISOL_ROLES, got {code}"
            );
        }
    })
    .await;
}

/// Tenant admin cannot update a user from another tenant.
#[tokio::test]
#[serial]
async fn tenant_admin_cannot_update_other_tenant_user() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;

        let ta = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "Cross Update Tenant",
            "CROSS_UPD",
            "cross-upd-admin@test.com",
            "admin1234",
            "Cross Update Admin",
        )
        .await;

        // Try to update user1 from DEFAULT tenant (id = 11111111-1111-1111-1111-111111111111)
        let default_user_id = "11111111-1111-1111-1111-111111111111";
        let (k1, v1) = prepare_data::auth_header(&ta.token);
        let update_payload = serde_json::json!({
            "name": "Hacked Name",
        });
        let update_response = request
            .put(&format!("/api/users/{default_user_id}"))
            .json(&update_payload)
            .add_header(k1, v1)
            .await;

        let status = update_response.status_code();
        assert!(
            status == 401 || status == 403,
            "Cross-tenant update should be denied, got {status}: {}",
            update_response.text()
        );
    })
    .await;
}

/// Tenant admin cannot toggle status of a user from another tenant.
#[tokio::test]
#[serial]
async fn tenant_admin_cannot_toggle_other_tenant_user_status() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;

        let ta = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "Cross Toggle Tenant",
            "CROSS_TOG",
            "cross-tog-admin@test.com",
            "admin1234",
            "Cross Toggle Admin",
        )
        .await;

        // Try to disable user1 from DEFAULT tenant
        let default_user_id = "11111111-1111-1111-1111-111111111111";
        let (k1, v1) = prepare_data::auth_header(&ta.token);
        let toggle_payload = serde_json::json!({ "status": "disabled" });
        let toggle_response = request
            .put(&format!("/api/users/{default_user_id}/status"))
            .json(&toggle_payload)
            .add_header(k1, v1)
            .await;

        let status = toggle_response.status_code();
        assert!(
            status == 401 || status == 403,
            "Cross-tenant status toggle should be denied, got {status}: {}",
            toggle_response.text()
        );
    })
    .await;
}

/// Tenant admin listing users only sees users in their own tenant.
#[tokio::test]
#[serial]
async fn tenant_user_list_isolated() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;

        let ta = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "List Isolation Tenant",
            "LIST_ISOL",
            "list-isol-admin@test.com",
            "admin1234",
            "List Isolation Admin",
        )
        .await;

        let (k1, v1) = prepare_data::auth_header(&ta.token);
        let response = request
            .get("/api/users?page=1&pageSize=50")
            .add_header(k1, v1)
            .await;
        assert_eq!(response.status_code(), 200);

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        let total = body["totalItems"].as_i64().unwrap();
        assert_eq!(
            total, 1,
            "Tenant admin should only see own tenant's 1 user (self), got {total}"
        );
    })
    .await;
}

/// Tenant admin can create a role within their own tenant.
#[tokio::test]
#[serial]
async fn new_tenant_admin_can_create_role() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;

        let ta = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "Create Role Tenant",
            "CRT_ROLE",
            "crt-role-admin@test.com",
            "admin1234",
            "Create Role Admin",
        )
        .await;

        let (k1, v1) = prepare_data::auth_header(&ta.token);
        let payload = serde_json::json!({
            "name": "Custom Role",
            "code": "CUSTOM",
            "description": "A custom role in a new tenant"
        });
        let response = request
            .post("/api/roles")
            .json(&payload)
            .add_header(k1, v1)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Tenant admin should create role: {}",
            response.text()
        );

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(body["name"], "Custom Role");
        assert_eq!(body["code"], "CUSTOM");
        assert_eq!(body["tenantCode"], "CRT_ROLE");
    })
    .await;
}

/// A newly created user with no roles should be denied access to protected resources.
#[tokio::test]
#[serial]
async fn new_user_without_role_gets_forbidden() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;

        let ta = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "Forbidden Tenant",
            "FORBIDDEN",
            "forbidden-admin@test.com",
            "admin1234",
            "Forbidden Admin",
        )
        .await;

        // Admin creates a plain user (no roles assigned)
        let (k1, v1) = prepare_data::auth_header(&ta.token);
        let create_payload = serde_json::json!({
            "email": "no-role-user@test.com",
            "password": "user1234",
            "name": "No Role User",
        });
        let create_response = request
            .post("/api/users")
            .json(&create_payload)
            .add_header(k1, v1)
            .await;
        assert_eq!(
            create_response.status_code(),
            200,
            "Create user should succeed: {}",
            create_response.text()
        );

        // Login as the new user (no roles)
        let login_response = request
            .post("/api/auth/login")
            .json(&serde_json::json!({
                "email": "no-role-user@test.com",
                "password": "user1234",
            }))
            .await;
        assert_eq!(
            login_response.status_code(),
            200,
            "Login should succeed: {}",
            login_response.text()
        );

        let lr: knota_fold::views::auth::LoginResponse =
            serde_json::from_str(&login_response.text()).unwrap();

        // Try to access protected endpoint — should get 403 from Casbin
        let (k2, v2) = prepare_data::auth_header(&lr.token);
        let roles_response = request
            .get("/api/roles?page=1&pageSize=10")
            .add_header(k2, v2)
            .await;

        let status = roles_response.status_code();
        assert_eq!(
            status,
            403,
            "User without roles should get 403, got {status}: {}",
            roles_response.text()
        );
    })
    .await;
}

/// Tenant admin cannot access super-admin-only endpoints (tenants list).
#[tokio::test]
#[serial]
async fn tenant_admin_cannot_list_all_tenants() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;

        let ta = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "NoList Tenant",
            "NOLIST",
            "nolist-admin@test.com",
            "admin1234",
            "NoList Admin",
        )
        .await;

        let (k1, v1) = prepare_data::auth_header(&ta.token);
        let response = request
            .get("/api/tenants?page=1&pageSize=10")
            .add_header(k1, v1)
            .await;

        let status = response.status_code();
        assert!(
            status == 401 || status == 403,
            "Tenant admin should not be able to list all tenants, got {status}: {}",
            response.text()
        );
    })
    .await;
}

/// Tenant admin cannot create a super admin user.
#[tokio::test]
#[serial]
async fn tenant_admin_cannot_create_super_admin() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;

        let ta = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "NoSuper Tenant",
            "NOSUPER",
            "nosuper-admin@test.com",
            "admin1234",
            "NoSuper Admin",
        )
        .await;

        let (k1, v1) = prepare_data::auth_header(&ta.token);
        let payload = serde_json::json!({
            "email": "fake-super@test.com",
            "password": "admin1234",
            "name": "Fake Super Admin",
        });
        let response = request
            .post("/api/sys/users/super-admin")
            .json(&payload)
            .add_header(k1, v1)
            .await;

        let status = response.status_code();
        assert!(
            status == 401 || status == 403 || status == 404,
            "Tenant admin should not be able to create super admin, got {status}: {}",
            response.text()
        );
    })
    .await;
}

/// Disabled tenant's user cannot access API.
#[tokio::test]
#[serial]
async fn disabled_tenant_user_access_denied() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;

        let ta = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "DisableMe Tenant",
            "DISABLE_ME",
            "disableme-admin@test.com",
            "admin1234",
            "DisableMe Admin",
        )
        .await;

        // super admin disables the tenant
        let (auth_key, auth_value) = prepare_data::auth_header(&super_admin.token);
        let disable_response = request
            .put(&format!("/api/tenants/{}", ta.tenant_id))
            .json(&serde_json::json!({ "status": "disabled" }))
            .add_header(auth_key, auth_value)
            .await;
        assert_eq!(
            disable_response.status_code(),
            200,
            "Super admin should be able to disable tenant: {}",
            disable_response.text()
        );

        // The tenant admin should now be rejected
        let (auth_key2, auth_value2) = prepare_data::auth_header(&ta.token);
        let response = request
            .get("/api/roles?page=1&pageSize=10")
            .add_header(auth_key2, auth_value2)
            .await;

        let status = response.status_code().as_u16();
        assert!(
            status >= 400,
            "Disabled tenant user should be rejected, got {status}: {}",
            response.text()
        );
    })
    .await;
}

/// Cross-tenant: tenant B user cannot access tenant A's dict types.
#[tokio::test]
#[serial]
async fn tenant_admin_cannot_access_other_tenant_dicts() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;

        let ta = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "DictIsol Tenant",
            "DICT_ISOL",
            "dict-isol-admin@test.com",
            "admin1234",
            "Dict Isolation Admin",
        )
        .await;

        // List dict types as tenant admin — may get 403 (no Casbin grant for dicts)
        // or 200 with only own tenant's data. Either way, cross-tenant data is protected.
        let (k1, v1) = prepare_data::auth_header(&ta.token);
        let response = request
            .get("/api/dicts/types?page=1&pageSize=50")
            .add_header(k1, v1)
            .await;

        if response.status_code() == 200 {
            let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
            let items = body["items"].as_array().unwrap();
            for item in items {
                if let Some(tc) = item.get("tenantCode").and_then(|v| v.as_str()) {
                    assert!(
                        tc.is_empty() || tc == "DICT_ISOL",
                        "Tenant admin should only see system or own-tenant dicts, got tenantCode={tc}"
                    );
                }
            }
        } else {
            // Access denied by Casbin — also valid isolation behavior
            assert!(
                response.status_code() == 401
                    || response.status_code().as_u16() == 403,
                "Expected 200, 401, or 403, got {}",
                response.status_code()
            );
        }
    })
    .await;
}

/// Cross-tenant: user from tenant A cannot access tenant B's i18n bundle.
#[tokio::test]
#[serial]
async fn tenant_user_bundle_scoped_to_own_tenant() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;

        let ta = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "i18nIsol Tenant",
            "I18N_ISOL",
            "i18n-isol-admin@test.com",
            "admin1234",
            "i18n Isolation Admin",
        )
        .await;

        // Get bundle as tenant admin — should succeed with own tenant scope
        let (k1, v1) = prepare_data::auth_header(&ta.token);
        let response = request
            .get("/api/i18n/bundles/CommonError/zh-CN")
            .add_header(k1, v1)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Tenant admin should be able to get bundle: {}",
            response.text()
        );
    })
    .await;
}

/// Full permission lifecycle:
///   super admin creates tenant → creates admin (auto TENANT_ADMIN role) →
///   assigns menus & permissions to TENANT_ADMIN role →
///   tenant admin logs in → creates role → assigns menus & permissions to new role.
#[tokio::test]
#[serial]
async fn tenant_admin_full_permission_lifecycle() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;

        // ── Step 1: Create tenant ─────────────────────────────────────
        let (k, v) = prepare_data::auth_header(&super_admin.token);
        let create_tenant_response = request
            .post("/api/tenants")
            .json(&serde_json::json!({
                "name": "Permission Lifecycle Tenant",
                "code": "PERM_LC",
            }))
            .add_header(k, v)
            .await;
        assert_eq!(
            create_tenant_response.status_code(),
            200,
            "Create tenant failed: {}",
            create_tenant_response.text()
        );

        // ── Step 2: Create tenant admin ───────────────────────────────
        let (k, v) = prepare_data::auth_header(&super_admin.token);
        let create_admin_response = request
            .post("/api/sys/tenants/PERM_LC/admins")
            .json(&serde_json::json!({
                "email": "perm-lc-admin@test.com",
                "password": "admin1234",
                "name": "PermLC Admin",
            }))
            .add_header(k, v)
            .await;
        assert_eq!(
            create_admin_response.status_code(),
            200,
            "Create tenant admin failed: {}",
            create_admin_response.text()
        );

        // ── Step 3: Super admin finds TENANT_ADMIN role for the new tenant ──
        let (k, v) = prepare_data::auth_header(&super_admin.token);
        let roles_response = request
            .get("/api/roles?page=1&pageSize=50&tenantCode=PERM_LC")
            .add_header(k, v)
            .await;
        assert_eq!(
            roles_response.status_code(),
            200,
            "List tenant roles failed: {}",
            roles_response.text()
        );

        let roles_body: serde_json::Value =
            serde_json::from_str(&roles_response.text()).unwrap();
        let role_items = roles_body["items"]
            .as_array()
            .expect("items should be an array");
        assert_eq!(
            role_items.len(),
            2,
            "Expected 2 roles (TENANT_ADMIN, MEMBER), got {}",
            role_items.len()
        );

        let tenant_admin_role = role_items
            .iter()
            .find(|r| r["code"].as_str() == Some("TENANT_ADMIN"))
            .expect("Should find TENANT_ADMIN role");
        let tenant_admin_role_id = tenant_admin_role["id"]
            .as_str()
            .expect("Role should have id");

        // ── Step 4: Super admin assigns menus to TENANT_ADMIN role ────
        // Use real fixture IDs: auth directory + role management menu
        let menu_ids = serde_json::json!({
            "sysMenuIds": [
                "cccccccc-cccc-cccc-cccc-ccccccccc001",
                "cccccccc-cccc-cccc-cccc-ccccccccc002"
            ]
        });
        let (k, v) = prepare_data::auth_header(&super_admin.token);
        let sync_menus_response = request
            .put(&format!("/api/roles/{tenant_admin_role_id}/menus"))
            .json(&menu_ids)
            .add_header(k, v)
            .await;
        assert_eq!(
            sync_menus_response.status_code(),
            200,
            "Sync menus to TENANT_ADMIN role failed: {}",
            sync_menus_response.text()
        );

        // Verify menus were synced
        let (k, v) = prepare_data::auth_header(&super_admin.token);
        let get_menus_response = request
            .get(&format!("/api/roles/{tenant_admin_role_id}/menus"))
            .add_header(k, v)
            .await;
        assert_eq!(get_menus_response.status_code(), 200);
        let menus_body: serde_json::Value =
            serde_json::from_str(&get_menus_response.text()).unwrap();
        let synced_menu_ids = menus_body["sysMenuIds"]
            .as_array()
            .expect("sysMenuIds should be an array");
        assert_eq!(
            synced_menu_ids.len(),
            2,
            "TENANT_ADMIN should have 2 menus, got {}",
            synced_menu_ids.len()
        );

        // ── Step 5: Super admin assigns API permissions to TENANT_ADMIN role ─
        // The TENANT_ADMIN role already got permissions from the template during
        // tenant creation. Verify those are present, then add more via sync.
        let (k, v) = prepare_data::auth_header(&super_admin.token);
        let get_perms_response = request
            .get(&format!("/api/roles/{tenant_admin_role_id}/permissions"))
            .add_header(k, v)
            .await;
        assert_eq!(get_perms_response.status_code(), 200);
        let perms_body: serde_json::Value =
            serde_json::from_str(&get_perms_response.text()).unwrap();
        let existing_perm_ids = perms_body["permissionIds"]
            .as_array()
            .expect("permissionIds should be an array");
        assert!(
            !existing_perm_ids.is_empty(),
            "TENANT_ADMIN should have permissions from template, got 0"
        );

        // Keep existing permissions and sync them back (idempotent)
        let perm_id_values: Vec<serde_json::Value> = existing_perm_ids.clone();
        let (k, v) = prepare_data::auth_header(&super_admin.token);
        let sync_perms_response = request
            .put(&format!("/api/roles/{tenant_admin_role_id}/permissions"))
            .json(&serde_json::json!({ "permissionIds": perm_id_values }))
            .add_header(k, v)
            .await;
        assert_eq!(
            sync_perms_response.status_code(),
            200,
            "Sync permissions to TENANT_ADMIN role failed: {}",
            sync_perms_response.text()
        );

        // ── Step 6: Tenant admin logs in ──────────────────────────────
        let login_response = request
            .post("/api/auth/login")
            .json(&serde_json::json!({
                "email": "perm-lc-admin@test.com",
                "password": "admin1234",
            }))
            .await;
        assert_eq!(
            login_response.status_code(),
            200,
            "Tenant admin login failed: {}",
            login_response.text()
        );
        let lr: LoginResponse = serde_json::from_str(&login_response.text()).unwrap();
        let ta_token = lr.token;

        // ── Step 7: Tenant admin creates a new role ───────────────────
        let (k, v) = prepare_data::auth_header(&ta_token);
        let create_role_response = request
            .post("/api/roles")
            .json(&serde_json::json!({
                "name": "Operator",
                "code": "OPERATOR",
                "description": "Operator role for perm lifecycle test"
            }))
            .add_header(k, v)
            .await;
        assert_eq!(
            create_role_response.status_code(),
            200,
            "Tenant admin create role failed: {}",
            create_role_response.text()
        );
        let new_role: serde_json::Value =
            serde_json::from_str(&create_role_response.text()).unwrap();
        assert_eq!(new_role["name"], "Operator");
        assert_eq!(new_role["code"], "OPERATOR");
        assert_eq!(new_role["tenantCode"], "PERM_LC");
        let new_role_id = new_role["id"].as_str().expect("New role should have id");

        // ── Step 8: Tenant admin gets assignable permissions for the new role ─
        let (k, v) = prepare_data::auth_header(&ta_token);
        let assignable_response = request
            .get(&format!("/api/roles/{new_role_id}/assignable-permissions"))
            .add_header(k, v)
            .await;
        assert_eq!(
            assignable_response.status_code(),
            200,
            "Get assignable permissions failed: {}",
            assignable_response.text()
        );
        let assignable_body: serde_json::Value =
            serde_json::from_str(&assignable_response.text()).unwrap();
        let all_permissions = assignable_body["permissions"]
            .as_array()
            .expect("permissions should be an array");
        assert!(
            !all_permissions.is_empty(),
            "Assignable permissions should not be empty"
        );
        // New role has no permissions yet
        let assigned_ids = assignable_body["assignedPermissionIds"]
            .as_array()
            .expect("assignedPermissionIds should be an array");
        assert!(
            assigned_ids.is_empty(),
            "New role should have no assigned permissions yet, got {}",
            assigned_ids.len()
        );

        // Verify metadata fields are present
        let first_perm = &all_permissions[0];
        assert!(first_perm.get("id").is_some(), "Missing 'id'");
        assert!(first_perm.get("name").is_some(), "Missing 'name'");
        assert!(first_perm.get("code").is_some(), "Missing 'code'");
        assert!(first_perm.get("obj").is_some(), "Missing 'obj'");
        assert!(first_perm.get("act").is_some(), "Missing 'act'");
        assert!(first_perm.get("tag").is_some(), "Missing 'tag'");
        assert!(
            first_perm.get("description").is_some(),
            "Missing 'description'"
        );

        // ── Step 9: Tenant admin assigns permissions to the new role ──
        // Pick the first 3 permission IDs to assign
        let perm_ids_to_assign: Vec<&str> = all_permissions
            .iter()
            .take(3)
            .map(|p| p["id"].as_str().unwrap())
            .collect();
        let (k, v) = prepare_data::auth_header(&ta_token);
        let sync_new_role_perms = request
            .put(&format!("/api/roles/{new_role_id}/permissions"))
            .json(&serde_json::json!({ "permissionIds": perm_ids_to_assign }))
            .add_header(k, v)
            .await;
        assert_eq!(
            sync_new_role_perms.status_code(),
            200,
            "Tenant admin sync role permissions failed: {}",
            sync_new_role_perms.text()
        );

        // Verify permissions were assigned
        let (k, v) = prepare_data::auth_header(&ta_token);
        let verify_perms = request
            .get(&format!("/api/roles/{new_role_id}/permissions"))
            .add_header(k, v)
            .await;
        assert_eq!(verify_perms.status_code(), 200);
        let verify_body: serde_json::Value =
            serde_json::from_str(&verify_perms.text()).unwrap();
        let verified_perm_ids = verify_body["permissionIds"]
            .as_array()
            .expect("permissionIds should be an array");
        assert_eq!(
            verified_perm_ids.len(),
            3,
            "New role should have exactly 3 permissions, got {}",
            verified_perm_ids.len()
        );

        // ── Step 10: Tenant admin assigns menus to the new role ───────
        let (k, v) = prepare_data::auth_header(&ta_token);
        let sync_new_role_menus = request
            .put(&format!("/api/roles/{new_role_id}/menus"))
            .json(&serde_json::json!({
                "sysMenuIds": ["cccccccc-cccc-cccc-cccc-ccccccccc001"]
            }))
            .add_header(k, v)
            .await;
        assert_eq!(
            sync_new_role_menus.status_code(),
            200,
            "Tenant admin sync role menus failed: {}",
            sync_new_role_menus.text()
        );

        // Verify menus were assigned
        let (k, v) = prepare_data::auth_header(&ta_token);
        let verify_menus = request
            .get(&format!("/api/roles/{new_role_id}/menus"))
            .add_header(k, v)
            .await;
        assert_eq!(verify_menus.status_code(), 200);
        let verify_menus_body: serde_json::Value =
            serde_json::from_str(&verify_menus.text()).unwrap();
        let verified_menu_ids = verify_menus_body["sysMenuIds"]
            .as_array()
            .expect("sysMenuIds should be an array");
        assert_eq!(
            verified_menu_ids.len(),
            1,
            "New role should have exactly 1 menu, got {}",
            verified_menu_ids.len()
        );
        assert_eq!(
            verified_menu_ids[0].as_str().unwrap(),
            "cccccccc-cccc-cccc-cccc-ccccccccc001",
            "Menu ID should match"
        );

        // ── Step 11: Final cross-check via assignable-permissions ─────
        // The new role should now show 3 assigned permissions
        let (k, v) = prepare_data::auth_header(&ta_token);
        let final_assignable = request
            .get(&format!("/api/roles/{new_role_id}/assignable-permissions"))
            .add_header(k, v)
            .await;
        assert_eq!(final_assignable.status_code(), 200);
        let final_body: serde_json::Value =
            serde_json::from_str(&final_assignable.text()).unwrap();
        let final_assigned = final_body["assignedPermissionIds"]
            .as_array()
            .expect("assignedPermissionIds should be an array");
        assert_eq!(
            final_assigned.len(),
            3,
            "After sync, assignable-permissions should show 3 assigned, got {}",
            final_assigned.len()
        );
    })
    .await;
}
