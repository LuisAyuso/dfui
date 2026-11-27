//! Bakes the AEMET key from the uncommitted `aemet.key` into the binary.
fn main() {
    println!("cargo:rerun-if-changed=aemet.key");
    if let Ok(key) = std::fs::read_to_string("aemet.key") {
        println!("cargo:rustc-env=AEMET_KEY_FILE={}", key.trim());
    }
}
