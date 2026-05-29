pub mod inbox;
pub mod manage;

use loco_openapi::prelude::*;
use loco_rs::prelude::*;

/// Management routes — Casbin-gated (create/list/revoke).
pub fn manage_routes() -> Routes {
    Routes::new()
        .prefix("/api/notifications")
        .add("/", openapi(post(manage::create), routes!(manage::create)))
        .add("/", openapi(get(manage::list), routes!(manage::list)))
        .add(
            "/{id}/revoke",
            openapi(put(manage::revoke), routes!(manage::revoke)),
        )
}

/// Inbox routes — all authenticated users, no Casbin.
pub fn inbox_routes() -> Routes {
    Routes::new()
        .prefix("/api/notifications")
        .add("/inbox", openapi(get(inbox::inbox), routes!(inbox::inbox)))
        .add(
            "/unread-count",
            openapi(get(inbox::unread_count), routes!(inbox::unread_count)),
        )
        .add(
            "/{id}/read",
            openapi(put(inbox::mark_read), routes!(inbox::mark_read)),
        )
        .add(
            "/read-all",
            openapi(put(inbox::mark_all_read), routes!(inbox::mark_all_read)),
        )
        .add(
            "/forced",
            openapi(get(inbox::forced), routes!(inbox::forced)),
        )
}
