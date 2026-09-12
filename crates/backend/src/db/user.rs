use serde::{Deserialize, Serialize};
use toasty::{Embed, Model, ModelSet};

#[derive(Model)]
pub struct User {
    #[key]
    #[auto]
    pub id: uuid::Uuid,

    #[unique]
    pub identity: String,
    pub password: String,

    pub name: Option<String>,
    pub role: UserRole,
}

#[derive(Embed, Serialize, Deserialize, utoipa::ToSchema)]
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
