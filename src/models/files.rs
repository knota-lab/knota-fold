pub use super::_entities::files::{self, ActiveModel, Column, Entity, Model};

// ActiveModelBehavior is provided by _entities/files.rs (default impl).
// Wave 2 will add before_save hook here for updated_at maintenance and ref_count guards.
