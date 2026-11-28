pub mod _entities;

impl sea_orm::ActiveModelBehavior for _entities::role_menus::ActiveModel {}
impl sea_orm::ActiveModelBehavior for _entities::role_permissions::ActiveModel {}
impl sea_orm::ActiveModelBehavior for _entities::user_roles::ActiveModel {}
impl sea_orm::ActiveModelBehavior for _entities::sys_role_template_menus::ActiveModel {}
impl sea_orm::ActiveModelBehavior
    for _entities::sys_role_template_permissions::ActiveModel
{
}
// Notification entities — ActiveModelBehavior is implemented in modules/notification/models/
// notification_recipients has no custom behavior, needs empty impl here.
impl sea_orm::ActiveModelBehavior for _entities::notification_recipients::ActiveModel {}
