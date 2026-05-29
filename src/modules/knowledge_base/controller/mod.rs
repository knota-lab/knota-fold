pub mod chat;
pub mod documents;
pub mod search;

use loco_openapi::prelude::*;
use loco_rs::prelude::*;

/// Management routes — Casbin-gated (document CRUD + reindex).
pub fn manage_routes() -> Routes {
    Routes::new()
        .prefix("/api/kb-documents")
        .add(
            "/",
            openapi(post(documents::create), routes!(documents::create)),
        )
        .add("/", openapi(get(documents::list), routes!(documents::list)))
        .add(
            "/{id}",
            openapi(get(documents::get), routes!(documents::get)),
        )
        .add(
            "/{id}",
            openapi(delete(documents::delete), routes!(documents::delete)),
        )
        .add(
            "/{id}/reindex",
            openapi(post(documents::reindex), routes!(documents::reindex)),
        )
        .add(
            "/{id}/promote",
            openapi(post(documents::promote), routes!(documents::promote)),
        )
}

/// User routes — JWT only, no Casbin (search + QA).
pub fn user_routes() -> Routes {
    Routes::new()
        .prefix("/api/kb")
        .add(
            "/search",
            openapi(post(search::search), routes!(search::search)),
        )
        .add(
            "/qa/v3/stream",
            openapi(post(search::qa_v3_stream), routes!(search::qa_v3_stream)),
        )
        .add(
            "/qa/v3/tool-result",
            openapi(
                post(search::receive_tool_result),
                routes!(search::receive_tool_result),
            ),
        )
        .add(
            "/documents/{id}/chunks",
            openapi(get(search::chunks), routes!(search::chunks)),
        )
}

/// Chat session routes — JWT only (session CRUD).
pub fn chat_routes() -> Routes {
    Routes::new()
        .prefix("/api/chat")
        .add(
            "/sessions",
            openapi(post(chat::create_session), routes!(chat::create_session)),
        )
        .add(
            "/sessions",
            openapi(get(chat::list_sessions), routes!(chat::list_sessions)),
        )
        .add(
            "/sessions/{id}",
            openapi(get(chat::get_session), routes!(chat::get_session)),
        )
        .add(
            "/sessions/{id}",
            openapi(delete(chat::delete_session), routes!(chat::delete_session)),
        )
        .add(
            "/sessions/{id}/export",
            openapi(get(chat::export_session), routes!(chat::export_session)),
        )
        .add(
            "/sessions/{id}/debug-export",
            openapi(
                get(chat::debug_export_session),
                routes!(chat::debug_export_session),
            ),
        )
}
