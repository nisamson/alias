use argon2::{self, Config, ThreadMode, Variant, Version};
use once_cell::sync::Lazy;
use rand::Rng;

pub static SECRET_KEY: Lazy<String> = Lazy::new(|| {
    std::env::var("ALIAS_SECRET_KEY").unwrap_or_else(|_| {
        eprintln!("Using empty secret key for password hashing. \
        Set ALIAS_SECRET_KEY= to silence this if intentional.");
        "".to_string()
    })
});

pub fn create_pass_hash(pass: impl AsRef<str>) -> String {
    static CONFIG: Lazy<Config> = Lazy::new(|| {
        Config {
            ad: &[],
            hash_length: 32,
            lanes: 4,
            mem_cost: 65536 >> 1,
            secret: SECRET_KEY.as_bytes(),
            thread_mode: ThreadMode::Parallel,
            time_cost: 10,
            variant: Variant::Argon2i,
            version: Version::Version13
        }
    });

    let salt = rand::thread_rng().gen::<[u8; 8]>();

    argon2::hash_encoded(pass.as_ref().as_bytes(), &salt, &CONFIG).unwrap()
}