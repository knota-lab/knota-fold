use loco_openapi::prelude::*;
use loco_rs::{
    app::{AppContext, Initializer},
    environment::Environment,
};
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_default();
        components.add_security_scheme(
            "bearerAuth",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("JWT or API Key")
                    .description(Some(
                        "Use `Authorization: Bearer <token>`. Knowledge-base endpoints accept either a user JWT or an API Key whose bound role grants the required permission.",
                    ))
                    .build(),
            ),
        );
    }
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Knota Fold API",
        description = "Knota Fold 后端 API"
    ),
    modifiers(&SecurityAddon)
)]
struct ApiDoc;

fn base_openapi() -> utoipa::openapi::OpenApi {
    ApiDoc::openapi()
}

#[must_use]
pub fn openapi_initializer(ctx: &AppContext) -> Option<Box<dyn Initializer>> {
    if ctx.environment == Environment::Test {
        return None;
    }

    #[allow(clippy::needless_for_each)]
    Some(Box::new(loco_openapi::OpenapiInitializerWithSetup::new(
        |_ctx| base_openapi(),
        None,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_spec_registers_bearer_auth() {
        let spec = base_openapi();
        let components = spec.components.expect("OpenAPI components");
        assert!(components.security_schemes.contains_key("bearerAuth"));
    }
}
