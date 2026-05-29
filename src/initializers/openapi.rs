use loco_openapi::prelude::*;
use loco_rs::{
    app::{AppContext, Initializer},
    environment::Environment,
};

pub fn openapi_initializer(ctx: &AppContext) -> Option<Box<dyn Initializer>> {
    if ctx.environment == Environment::Test {
        return None;
    }

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
