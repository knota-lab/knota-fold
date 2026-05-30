use loco_openapi::prelude::*;
use loco_rs::{
    app::{AppContext, Initializer},
    environment::Environment,
};

#[must_use]
pub fn openapi_initializer(ctx: &AppContext) -> Option<Box<dyn Initializer>> {
    if ctx.environment == Environment::Test {
        return None;
    }

    #[allow(clippy::needless_for_each)]
    Some(Box::new(loco_openapi::OpenapiInitializerWithSetup::new(
        |_ctx| {
            #[derive(OpenApi)]
            #[openapi(info(
                title = "Knota Fold API",
                description = "Knota Fold 后端 API"
            ))]
            struct ApiDoc;
            ApiDoc::openapi()
        },
        None,
    )))
}
