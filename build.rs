//! Builds the bundled plugins in `plugins/` as WebAssembly components and copies
//! them into `OUT_DIR`, where `src/plugins.rs` embeds them with `include_bytes!`.

use std::{env, path::PathBuf, process::Command};

const PLUGIN_TARGET: &str = "wasm32-wasip2";
/// Plugins to bundle: (crate name, directory under `plugins/`). Core comes first
/// so the others can bind its commands.
const BUNDLED: &[(&str, &str)] = &[("arrow-core", "core"), ("text-tools", "text-tools")];

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    // A separate target dir so this nested build never waits on the outer one's lock.
    let target_dir = manifest_dir.join("target").join("plugins");
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());

    println!("cargo:rerun-if-changed=wit");
    for (plugin, dir) in BUNDLED {
        println!("cargo:rerun-if-changed=plugins/{dir}/src");
        println!("cargo:rerun-if-changed=plugins/{dir}/Cargo.toml");

        let status = Command::new(&cargo)
            .args(["build", "--release", "--target", PLUGIN_TARGET, "--package", plugin])
            .arg("--target-dir")
            .arg(&target_dir)
            .current_dir(&manifest_dir)
            // Flags meant for the host build must not leak into the wasm build.
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env_remove("CARGO_BUILD_TARGET")
            .status()
            .expect("failed to run cargo to build a bundled plugin");
        if !status.success() {
            panic!(
                "building plugin `{plugin}` failed. Is the target installed? \
                 Run `rustup target add {PLUGIN_TARGET}`."
            );
        }

        let file = format!("{}.wasm", plugin.replace('-', "_"));
        let wasm = target_dir.join(PLUGIN_TARGET).join("release").join(&file);
        std::fs::copy(&wasm, out_dir.join(&file))
            .unwrap_or_else(|e| panic!("could not copy {}: {e}", wasm.display()));
    }
}
