use std::{env, path::PathBuf};

fn main() {
    let manifest_path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let msg_path = manifest_path.join("std_msgs");
    println!(
        "cargo:rustc-env=ROSRUST_MSG_PATH={}",
        msg_path.to_string_lossy(),
    );
}
