use toasty::{Deferred, Model, ModelSet};

use crate::db::user::User;

#[derive(Model)]
pub struct Session {
    #[key]
    #[auto]
    pub id: uuid::Uuid,

    #[unique]
    pub user_id: uuid::Uuid,
    #[belongs_to(key = user_id, references = id)]
    pub user: Deferred<User>,

    #[unique]
    pub token: String,
    pub expires_at: jiff::Timestamp,
}

pub(super) fn models() -> ModelSet {
    toasty::models!(crate::*)
}
