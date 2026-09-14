use serde::{Deserialize, Serialize};
use toasty::{Deferred, Embed, Model, ModelSet};

use crate::db::auth::Session;

#[derive(Model)]
pub struct User {
    #[key]
    #[auto]
    pub id: uuid::Uuid,

    pub name: Option<String>,
    pub role: UserRole,

    #[unique]
    pub identity: String,
    pub password: String,

    #[has_one]
    pub session: Deferred<Option<Session>>,
}

#[derive(Embed, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[column(type = u8)]
pub enum UserRole {
    #[column(variant = 0)]
    Admin,
    #[column(variant = 1)]
    Teacher,
    #[column(variant = 2)]
    Student,
}

pub(super) fn models() -> ModelSet {
    toasty::models!(crate::*)
}
