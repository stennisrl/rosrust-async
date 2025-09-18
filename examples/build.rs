use std::{env, path::PathBuf};

fn main() {
    let manifest_path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    println!(
        "cargo:rustc-env=ROSRUST_MSG_PATH={}",
        manifest_path.join("msgs").to_string_lossy(),
    );
}
