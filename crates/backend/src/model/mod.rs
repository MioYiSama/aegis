use toasty::ModelSet;

pub use session::Session;
pub use user::{User, UserRole};

mod session;
mod user;

pub fn models() -> ModelSet {
    vec![session::models(), user::models()]
        .into_iter()
        .flatten()
        .fold(ModelSet::new(), |mut models, model| {
            models.add(model);
            models
        })
}
