use toasty::{Deferred, Model, ModelSet, models};
use uuid::Uuid;

use crate::model::User;

pub fn models() -> ModelSet {
    models!(crate::*)
}

#[derive(Model)]
pub struct Session {
    #[key]
    #[auto]
    pub id: Uuid,

    #[unique]
    pub token: String,
    pub expires_at: jiff::Timestamp,

    #[unique]
    pub user_id: Uuid,
    #[belongs_to(key = user_id, references = id)]
    pub user: Deferred<User>,
}
