pub mod notification_recipients;
pub mod notifications;

// Re-export common types for convenience within the module.
pub use notification_recipients::{
    ActiveModel as RecipientActiveModel, Model as RecipientModel,
};
pub use notifications::{
    ActiveModel as NotificationActiveModel, Model as NotificationModel,
};
