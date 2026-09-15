pub mod handler;
pub mod model;
pub mod service;
pub mod state;

pub const DEBUG: bool = cfg!(debug_assertions);
pub const RELEASE: bool = cfg!(not(debug_assertions));

pub fn init_tracing() {
    tracing_subscriber::fmt().init();
}

pub fn load_dotenv() {
    let env_file = if cfg!(debug_assertions) {
        ".env.development"
    } else {
        ".env.production"
    };

    println!("Loading {env_file}");
    _ = dotenvy::from_filename(env_file);
}
