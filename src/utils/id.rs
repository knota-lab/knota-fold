use uuid::Uuid;

#[must_use]
pub fn generate_id() -> Uuid {
    Uuid::now_v7()
}
