use async_trait::async_trait;
use axum::Router as AxumRouter;
use loco_rs::prelude::*;
use utoipa::openapi::OpenApi;
use utoipa_axum::router::OpenApiRouter;
#[cfg(feature = "redoc")]
use utoipa_redoc::{Redoc, Servable};
#[cfg(feature = "scalar")]
use utoipa_scalar::{Scalar, Servable as ScalarServable};
#[cfg(feature = "swagger")]
use utoipa_swagger_ui::SwaggerUi;

use crate::config::{get_openapi_config, set_openapi_config, InitializerConfig};
use crate::openapi::get_merged_router;
// Always used
use crate::utils::set_openapi_spec;
// Only used in feature blocks
#[cfg(any(feature = "redoc", feature = "scalar", feature = "swagger"))]
use crate::utils::{add_openapi_endpoints, get_openapi_spec};

pub mod auth;
pub mod config;
pub mod openapi;
pub mod prelude;
pub mod utils;

type RouterList = Option<Vec<OpenApiRouter<AppContext>>>;
type InitialSpec = dyn Fn(&AppContext) -> OpenApi + Send + Sync + 'static;

/// Loco initializer for `OpenAPI` with custom initial spec setup
#[derive(Default)]
pub struct OpenapiInitializerWithSetup {
    /// Custom setup for the initial `OpenAPI` spec, if any
    initial_spec: Option<Box<InitialSpec>>,
    /// Routes to add to the `OpenAPI` spec
    routes_setup: RouterList,
}

impl OpenapiInitializerWithSetup {
    #[must_use]
    pub fn new<F>(initial_spec: F, routes_setup: RouterList) -> Self
    where
        F: Fn(&AppContext) -> OpenApi + Send + Sync + 'static,
    {
        Self {
            initial_spec: Some(Box::new(initial_spec)),
            routes_setup,
        }
    }
}

#[async_trait]
impl Initializer for OpenapiInitializerWithSetup {
    fn name(&self) -> String {
        "openapi".to_string()
    }

    async fn after_routes(
        &self,
        router: AxumRouter,
        ctx: &AppContext,
    ) -> Result<AxumRouter> {
        // Use the InitializerConfig wrapper
        set_openapi_config(InitializerConfig::from(&ctx.config.initializers).into())?;

        let mut api_router: OpenApiRouter<AppContext> = self
            .initial_spec
            .as_ref()
            .map_or_else(OpenApiRouter::new, |custom_spec_fn| {
                OpenApiRouter::with_openapi(custom_spec_fn(ctx))
            });

        // Merge all manually collected routes
        if let Some(ref routes_setup) = self.routes_setup {
            for route in routes_setup {
                api_router = api_router.merge(route.clone());
            }
        }

        // Merge all automatically collected routes
        api_router = api_router.merge(get_merged_router());

        // Collect the `OpenAPI` spec
        let (_, open_api_spec) = api_router.split_for_parts();
        set_openapi_spec(open_api_spec);

        // Use `_` prefix as config might be unused if no features are enabled
        let Some(_open_api_config) = get_openapi_config() else {
            // No config, return original router
            return Ok(router);
        };

        // Create a new router for UI endpoints
        #[allow(unused_mut)]
        let mut ui_router = AxumRouter::new();

        // Serve the `OpenAPI` spec using the enabled `OpenAPI` visualizers
        #[cfg(feature = "redoc")]
        if let Some(config::OpenAPIType::Redoc {
            url,
            spec_json_url,
            spec_yaml_url,
        }) = get_openapi_config().and_then(|c| c.redoc.as_ref())
        {
            ui_router = ui_router.merge(Redoc::with_url(url, get_openapi_spec().clone()));
            ui_router = add_openapi_endpoints(ui_router, spec_json_url, spec_yaml_url);
        }

        #[cfg(feature = "scalar")]
        if let Some(config::OpenAPIType::Scalar {
            url,
            spec_json_url,
            spec_yaml_url,
        }) = get_openapi_config().and_then(|c| c.scalar.as_ref())
        {
            ui_router =
                ui_router.merge(Scalar::with_url(url, get_openapi_spec().clone()));
            ui_router = add_openapi_endpoints(ui_router, spec_json_url, spec_yaml_url);
        }

        #[cfg(feature = "swagger")]
        if let Some(config::OpenAPIType::Swagger {
            url,
            spec_json_url,
            spec_yaml_url,
        }) = get_openapi_config().and_then(|c| c.swagger.as_ref())
        {
            ui_router = ui_router.merge(
                SwaggerUi::new(url)
                    .url(spec_json_url.clone(), get_openapi_spec().clone()),
            );
            ui_router = add_openapi_endpoints(ui_router, &None, spec_yaml_url);
        }

        // Merge the UI router with the main router
        Ok(router.merge(ui_router))
    }
}
