use async_trait::async_trait;
use axum::Router as AxumRouter;
use loco_rs::{
    app::{AppContext, Initializer},
    Result,
};

use crate::services::casbin_service;

pub struct CasbinInitializer;

#[async_trait]
impl Initializer for CasbinInitializer {
    fn name(&self) -> String {
        "casbin".to_string()
    }

    async fn before_run(&self, app_context: &AppContext) -> Result<()> {
        let enforcer = casbin_service::init_enforcer(&app_context.db).await?;
        app_context.shared_store.insert(enforcer);
        tracing::info!("Casbin enforcer initialized and stored in shared_store");
        Ok(())
    }

    async fn after_routes(
        &self,
        router: AxumRouter,
        _ctx: &AppContext,
    ) -> Result<AxumRouter> {
        Ok(router)
    }
}
