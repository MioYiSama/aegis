use serde::{Deserialize, Serialize};
use toasty::{Deferred, Embed, Model, ModelSet, models};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::model::Session;

pub fn models() -> ModelSet {
    models!(crate::*)
}

#[derive(Model)]
pub struct User {
    #[key]
    #[auto]
    pub id: Uuid,

    #[unique]
    pub identity: String,
    pub password: String,

    #[has_one]
    pub session: Deferred<Option<Session>>,

    pub name: Option<String>,
    pub role: UserRole,
}

#[derive(Embed, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[column(type = u8)]
pub enum UserRole {
    #[column(variant = 0)]
    Admin,
    #[column(variant = 1)]
    Teacher,
    #[column(variant = 2)]
    Student,
}
