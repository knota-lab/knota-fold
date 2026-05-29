use uuid::Uuid;

pub fn generate_id() -> Uuid {
    Uuid::now_v7()
}
