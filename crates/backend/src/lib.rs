pub mod db;
pub mod routes;
pub mod state;

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
