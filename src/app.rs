use async_trait::async_trait;
use axum::{response::IntoResponse, Router as AxumRouter};
use loco_rs::{
    app::{AppContext, Hooks, Initializer},
    bgworker::{BackgroundWorker, Queue},
    boot::{create_app, BootResult, ServeParams, StartMode},
    config::Config,
    controller::AppRoutes,
    db::{self, truncate_table},
    environment::Environment,
    task::Tasks,
    Error, Result,
};
use migration::Migrator;
use std::{net::TcpListener, path::Path};

#[allow(unused_imports)]
use crate::{
    controllers,
    middleware::casbin_authz::CasbinAuthzLayer,
    middleware::ci_token_layer::CiTokenLayer,
    middleware::error_handler::ErrorHandlerLayer,
    middleware::tracing::TracingLayer,
    models::_entities::{
        api_key_exchange_tokens, api_keys, audit_logs, dict_items, dict_types,
        file_references, file_upload_idempotency, file_upload_parts, file_uploads, files,
        kb_folders, kb_libraries, notification_recipients, notifications, permissions,
        role_menus, role_permissions, roles, scheduled_worker_definitions,
        scheduled_worker_executions, scheduled_worker_schedules,
        scheduled_worker_tenant_grants, sys_configs, sys_menus, sys_role_template_menus,
        sys_role_template_permissions, sys_role_templates, tenant_menu_overrides,
        tenants, user_roles, users,
    },
    services::casbin_service::SharedEnforcer,
    tasks,
    workers::{
        downloader::DownloadWorker, indexing_worker::IndexingWorker,
        test_job_worker::TestJobWorker,
    },
};

fn casbin_authz_layer(ctx: &AppContext) -> CasbinAuthzLayer {
    let enforcer = ctx
        .shared_store
        .get::<SharedEnforcer>()
        .expect("Casbin enforcer not initialized");
    let jwt_secret = ctx
        .config
        .get_jwt_config()
        .expect("JWT config missing")
        .secret
        .clone();

    CasbinAuthzLayer::new(enforcer, ctx.db.clone(), jwt_secret, ctx.cache.clone())
}

pub struct App;
#[async_trait]
impl Hooks for App {
    fn app_name() -> &'static str {
        env!("CARGO_CRATE_NAME")
    }

    fn app_version() -> String {
        format!(
            "{} ({})",
            env!("CARGO_PKG_VERSION"),
            option_env!("BUILD_SHA")
                .or(option_env!("GITHUB_SHA"))
                .unwrap_or("dev")
        )
    }

    fn init_logger(ctx: &AppContext) -> Result<bool> {
        crate::app_logs::init_logger(ctx)
    }

    async fn before_run(ctx: &AppContext) -> Result<()> {
        check_start_server_port_before_loco_banner(ctx)
    }

    async fn boot(
        mode: StartMode,
        environment: &Environment,
        config: Config,
    ) -> Result<BootResult> {
        create_app::<Self, Migrator>(mode, environment, config).await
    }

    async fn serve(
        app: AxumRouter,
        _ctx: &AppContext,
        serve_params: &ServeParams,
    ) -> Result<()> {
        let addr = format!("{}:{}", serve_params.binding, serve_params.port);
        let listener = tokio::net::TcpListener::bind(&addr).await.map_err(|err| {
            Error::Message(format!(
                "server address {addr} is not available: {err}. Stop the process using this port or change `server.binding` / `server.port`."
            ))
        })?;

        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let () = loco_rs::boot::shutdown_signal().await;
        })
        .await?;
        Ok(())
    }

    async fn initializers(ctx: &AppContext) -> Result<Vec<Box<dyn Initializer>>> {
        let mut initializers: Vec<Box<dyn Initializer>> = vec![
            Box::new(crate::initializers::config_validator::ConfigValidator),
            Box::new(crate::initializers::sqlite_wal::SqliteWalInitializer),
            Box::new(crate::initializers::casbin::CasbinInitializer),
            Box::new(crate::initializers::s3::S3ClientInitializer),
            Box::new(crate::initializers::knowledge_base::KnowledgeBaseInitializer),
            Box::new(crate::app_logs::AppLogsInitializer),
        ];

        if let Some(openapi) = crate::initializers::openapi::openapi_initializer(ctx) {
            initializers.push(openapi);
        }

        Ok(initializers)
    }

    fn routes(ctx: &AppContext) -> AppRoutes {
        let authz_layer = casbin_authz_layer(ctx);

        AppRoutes::with_default_routes() // controller routes below
            .add_route(controllers::auth::routes())
            .add_route(controllers::auth::super_admin_routes().layer(authz_layer.clone()))
            .add_route(controllers::roles::routes().layer(authz_layer.clone()))
            .add_route(controllers::roles::user_role_routes().layer(authz_layer.clone()))
            .add_route(controllers::permissions::routes().layer(authz_layer.clone()))
            .add_route(controllers::sys_menus::routes().layer(authz_layer.clone()))
            .add_route(controllers::menus::routes().layer(authz_layer.clone()))
            .add_route(controllers::menus::user_menu_routes().layer(authz_layer.clone()))
            .add_route(controllers::dicts::dict_type_routes().layer(authz_layer.clone()))
            .add_route(controllers::dicts::dict_item_routes().layer(authz_layer.clone()))
            .add_route(controllers::users::routes().layer(authz_layer.clone()))
            .add_route(controllers::tenants::routes().layer(authz_layer.clone()))
            .add_route(controllers::tenants::sys_routes().layer(authz_layer.clone()))
            .add_route(controllers::role_templates::routes().layer(authz_layer.clone()))
            .add_route(controllers::audit_logs::routes().layer(authz_layer.clone()))
            .add_route(controllers::sys_configs::routes().layer(authz_layer.clone()))
            .add_route(
                controllers::sys_configs::tenant_routes().layer(authz_layer.clone()),
            )
            .add_route(
                controllers::sys_configs::super_admin_routes().layer(authz_layer.clone()),
            )
            .add_route(
                controllers::sys_configs::resolved_routes().layer(authz_layer.clone()),
            )
            .add_route(controllers::files::routes().layer(authz_layer.clone()))
            .add_route(controllers::file_uploads::routes().layer(authz_layer.clone()))
            .add_route(
                controllers::file_references::routes_files_subpath()
                    .layer(authz_layer.clone()),
            )
            .add_route(
                controllers::file_references::routes_root().layer(authz_layer.clone()),
            )
            .add_route(controllers::sys_files::routes().layer(authz_layer.clone()))
            .add_route(controllers::sys_file_uploads::routes().layer(authz_layer.clone()))
            .add_route(
                controllers::sys_file_references::routes_files_subpath()
                    .layer(authz_layer.clone()),
            )
            .add_route(
                controllers::sys_file_references::routes_root()
                    .layer(authz_layer.clone()),
            )
            // i18n user-facing reads — JWT only (TenantContext), no Casbin.
            // Every logged-in user needs locales + bundles to render any page.
            .add_route(controllers::i18n::user_routes())
            // i18n admin (global locales + global translations) — Casbin-gated.
            .add_route(controllers::admin_i18n::routes().layer(authz_layer.clone()))
            // i18n tenant overrides — current tenant via JWT.
            .add_route(
                controllers::tenant_i18n::tenant_routes().layer(authz_layer.clone()),
            )
            // CI endpoints — gated by X-CI-Token (env-provisioned), NOT Casbin.
            .add_route(controllers::ci_i18n::routes().layer(CiTokenLayer::new()))
            // Public i18n bundles — no auth at all (Login page etc.).
            .add_route(controllers::i18n::public_routes())
            // App logs admin API — Casbin-gated.
            .add_route(crate::app_logs::routes::routes().layer(authz_layer.clone()))
            // Task scheduler — Casbin-gated.
            .add_route(
                controllers::worker_definitions::routes().layer(authz_layer.clone()),
            )
            .add_route(controllers::worker_schedules::routes().layer(authz_layer.clone()))
            .add_route(
                controllers::worker_executions::routes().layer(authz_layer.clone()),
            )
            .add_route(controllers::api_keys::routes().layer(authz_layer.clone()))
            .add_route(
                controllers::api_key_exchange_tokens::routes().layer(authz_layer.clone()),
            )
            .add_route(controllers::api_key_exchange_tokens::public_routes())
            // Notification module — manages routes (create/list/revoke) Casbin-gated;
            // inbox/read/unread/forced are login-only.
            .add_route(
                crate::modules::notification::controller::manage_routes()
                    .layer(authz_layer.clone()),
            )
            .add_route(crate::modules::notification::controller::inbox_routes())
            // Knowledge base — manage routes (Casbin-gated CRUD)
            .add_route(
                crate::modules::knowledge_base::controller::manage_routes()
                    .layer(authz_layer.clone()),
            )
            .add_route(
                crate::modules::knowledge_base::controller::library_routes()
                    .layer(authz_layer.clone()),
            )
            .add_route(
                crate::modules::knowledge_base::controller::folder_routes()
                    .layer(authz_layer.clone()),
            )
            // Knowledge base — reads + QA shared by JWT and API Key clients.
            .add_route(
                crate::modules::knowledge_base::controller::access_routes()
                    .layer(authz_layer),
            )
            // Frontend Agent bridge — JWT only.
            .add_route(crate::modules::knowledge_base::controller::user_routes())
            // Chat sessions — JWT only (session CRUD)
            .add_route(crate::modules::knowledge_base::controller::chat_routes())
    }
    async fn after_routes(
        router: axum::Router,
        _ctx: &AppContext,
    ) -> Result<axum::Router> {
        // TracingLayer is the outermost layer: it wraps Casbin auth and all
        // controllers, ensuring every request gets a trace_id and request_id
        // before any downstream middleware runs.
        crate::services::file_service::init_runtime(_ctx)?;

        // Override loco-rs's default HTML fallback for /api/* routes.
        //
        // loco-rs registers a catch-all fallback that returns HTML (200 OK) for
        // ALL unmatched paths — intended for SPA support. This is confusing for
        // API clients because a typo in an API path returns HTML 200 instead of
        // JSON 404. We replace the fallback: /api/* → JSON 404, everything else
        // → the original loco-rs HTML page.
        let fallback_html = include_str!("fallback.html").to_owned();
        let router = router.fallback(move |req: axum::extract::Request| {
            let html = fallback_html.clone();
            async move {
                let path = req.uri().path();
                if path.starts_with("/api/") || path == "/api" {
                    (
                        axum::http::StatusCode::NOT_FOUND,
                        [(axum::http::header::CONTENT_TYPE, "application/json")],
                        r#"{"error":"not found"}"#,
                    )
                        .into_response()
                } else {
                    (axum::http::StatusCode::OK, axum::response::Html(html))
                        .into_response()
                }
            }
        });

        // ErrorHandlerLayer sits inside TracingLayer: it patches error JSON
        // responses (format B → format A) so the frontend always sees a
        // standalone `code` field. TracingLayer wraps everything for spans.
        Ok(router.layer(ErrorHandlerLayer).layer(TracingLayer))
    }

    async fn connect_workers(ctx: &AppContext, queue: &Queue) -> Result<()> {
        queue.register(DownloadWorker::build(ctx)).await?;
        queue.register(TestJobWorker::build(ctx)).await?;
        queue.register(IndexingWorker::build(ctx)).await?;
        Ok(())
    }

    fn register_tasks(tasks: &mut Tasks) {
        tasks.register(tasks::bootstrap_admin::BootstrapAdmin);
        tasks.register(tasks::purge_files::PurgeFiles);
        tasks.register(tasks::purge_uploads::PurgeUploads);
        tasks.register(tasks::scheduler_dispatch::SchedulerDispatch);
        tasks.register(tasks::seed_error_codes::SeedErrorCodes);
        tasks.register(tasks::test_job::TestJob);
        // tasks-inject (do not remove)
    }
    async fn truncate(ctx: &AppContext) -> Result<()> {
        truncate_table(&ctx.db, file_references::Entity).await?;
        truncate_table(&ctx.db, file_upload_idempotency::Entity).await?;
        truncate_table(&ctx.db, file_upload_parts::Entity).await?;
        truncate_table(&ctx.db, file_uploads::Entity).await?;
        truncate_table(&ctx.db, files::Entity).await?;
        truncate_table(&ctx.db, audit_logs::Entity).await?;
        truncate_table(&ctx.db, user_roles::Entity).await?;
        truncate_table(&ctx.db, role_menus::Entity).await?;
        truncate_table(&ctx.db, role_permissions::Entity).await?;
        truncate_table(&ctx.db, users::Entity).await?;
        truncate_table(&ctx.db, roles::Entity).await?;
        truncate_table(&ctx.db, tenant_menu_overrides::Entity).await?;
        truncate_table(&ctx.db, permissions::Entity).await?;
        truncate_table(&ctx.db, sys_menus::Entity).await?;
        truncate_table(&ctx.db, dict_types::Entity).await?;
        truncate_table(&ctx.db, dict_items::Entity).await?;
        truncate_table(&ctx.db, sys_role_template_menus::Entity).await?;
        truncate_table(&ctx.db, sys_role_template_permissions::Entity).await?;
        truncate_table(&ctx.db, sys_role_templates::Entity).await?;
        truncate_table(&ctx.db, scheduled_worker_definitions::Entity).await?;
        truncate_table(&ctx.db, scheduled_worker_tenant_grants::Entity).await?;
        truncate_table(&ctx.db, scheduled_worker_schedules::Entity).await?;
        truncate_table(&ctx.db, scheduled_worker_executions::Entity).await?;
        truncate_table(&ctx.db, api_keys::Entity).await?;
        truncate_table(&ctx.db, api_key_exchange_tokens::Entity).await?;
        truncate_table(&ctx.db, notification_recipients::Entity).await?;
        truncate_table(&ctx.db, notifications::Entity).await?;
        truncate_table(&ctx.db, kb_folders::Entity).await?;
        truncate_table(&ctx.db, kb_libraries::Entity).await?;
        truncate_table(&ctx.db, tenants::Entity).await?;
        truncate_table(&ctx.db, sys_configs::Entity).await?;
        Ok(())
    }

    async fn seed(ctx: &AppContext, base: &Path) -> Result<()> {
        if ctx.environment != Environment::Test {
            tracing::info!(
                "default user fixtures are disabled outside tests; run `task bootstrap_admin` to create the first administrator"
            );
            return Ok(());
        }

        db::seed::<users::ActiveModel>(
            &ctx.db,
            &base.join("users.yaml").display().to_string(),
        )
        .await?;
        db::seed::<user_roles::ActiveModel>(
            &ctx.db,
            &base.join("user_roles.yaml").display().to_string(),
        )
        .await?;
        Ok(())
    }
}

fn check_start_server_port_before_loco_banner(ctx: &AppContext) -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !is_start_command(&args) || is_worker_only_start(&args) {
        return Ok(());
    }

    let binding = find_option_value(&args, "--binding", "-b")
        .unwrap_or_else(|| ctx.config.server.binding.clone());
    let port = find_option_value(&args, "--port", "-p")
        .map_or(ctx.config.server.port, |value| {
            value.parse::<i32>().unwrap_or(ctx.config.server.port)
        });
    let addr = format!("{binding}:{port}");

    let listener = TcpListener::bind(&addr).map_err(|err| {
        Error::Message(format!(
            "server address {addr} is not available before startup: {err}. Stop the process using this port or change `server.binding` / `server.port`."
        ))
    })?;
    drop(listener);
    Ok(())
}

fn is_start_command(args: &[String]) -> bool {
    first_command(args).is_some_and(|command| command == "start" || command == "s")
}

fn is_worker_only_start(args: &[String]) -> bool {
    has_flag(args, "--worker", "-w")
        && !has_flag(args, "--server-and-worker", "")
        && !has_flag(args, "--all", "")
}

fn first_command(args: &[String]) -> Option<&str> {
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if option_takes_value(arg) {
            skip_next = !arg.contains('=');
            continue;
        }
        if !arg.starts_with('-') {
            return Some(arg.as_str());
        }
    }
    None
}

fn option_takes_value(arg: &str) -> bool {
    matches!(
        arg,
        "-e" | "--environment" | "-b" | "--binding" | "-p" | "--port"
    ) || arg.starts_with("--environment=")
        || arg.starts_with("--binding=")
        || arg.starts_with("--port=")
}

fn find_option_value(args: &[String], long: &str, short: &str) -> Option<String> {
    let long_eq = format!("{long}=");
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if let Some(value) = arg.strip_prefix(&long_eq) {
            return Some(value.to_string());
        }
        if arg == long || (!short.is_empty() && arg == short) {
            return iter.next().cloned();
        }
    }
    None
}

fn has_flag(args: &[String], long: &str, short: &str) -> bool {
    let long_eq = format!("{long}=");
    args.iter().any(|arg| {
        arg == long || (!short.is_empty() && arg == short) || arg.starts_with(&long_eq)
    })
}
