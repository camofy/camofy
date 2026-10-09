pub mod compatibility;
pub mod config_compat;
pub mod config_compatibility;
pub mod engine;
pub mod node_filter;
pub mod protocol;

pub fn digest(bytes: impl AsRef<[u8]>) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}
