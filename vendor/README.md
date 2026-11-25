# Vendored dependency forks

These two crates are pinned here because their published releases still depend on
pre-1.0 Loco / SeaORM. Remove them once upstream ships compatible versions.

## `loco-openapi` 0.1.2

- Upstream: <https://github.com/loco-rs/loco-openapi-Initializer> (last commit 2025-08)
- Patch: `loco-rs` dependency `0.16` → `1.3` (source unchanged; compiles against
  Loco 1.3 with axum 0.8).
- Consumed by ~37 controller/route files through `loco_openapi::prelude::*` and
  the `openapi(...)` / `routes!(...)` helpers.

## `sea-orm-adapter` 0.4.0

- Upstream: <https://github.com/ZihanType/sea-orm-adapter> (last commit 2024-08)
- Patch: `sea-orm` dependency `1` → `2`, two `ConnectionTrait::execute` calls
  became `execute_raw`, and the removed actix runtime features were dropped.
- Used by `src/services/casbin_service.rs` (`SeaOrmAdapter::new`).

Both crates are excluded from the Cargo workspace (`[workspace].exclude`) so
`cargo clippy -- -D warnings` only lints this repository's code.
