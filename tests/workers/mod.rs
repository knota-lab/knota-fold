use knota_fold::app::App;
use knota_fold::models::scheduled_worker_definitions;
use loco_rs::testing::prelude::*;
use sea_orm::EntityTrait;
use serial_test::serial;

#[tokio::test]
#[serial]
async fn seed_data_includes_worker_definitions() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let definitions = scheduled_worker_definitions::Entity::find()
        .all(db)
        .await
        .expect("Failed to query worker definitions");

    assert!(
        !definitions.is_empty(),
        "Seed data should include at least one worker definition"
    );

    let test_job = definitions.iter().find(|d| d.code == "test_job");
    assert!(
        test_job.is_some(),
        "test_job worker definition should be seeded"
    );
}
