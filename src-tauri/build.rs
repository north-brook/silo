use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const OBSERVER_TARGET: &str = "x86_64-unknown-linux-musl";
const OBSERVER_OUTPUT_NAME: &str = "workspace-observer-x86_64-unknown-linux-musl";

fn main() {
    build_workspace_observer();
    tauri_build::build()
}

fn build_workspace_observer() {
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .expect("src-tauri should live under the workspace root");
    let observer_dir = workspace_root.join("tools/workspace-observer");
    let observer_manifest = observer_dir.join("Cargo.toml");
    let observer_lock = observer_dir.join("Cargo.lock");
    let observer_main = observer_dir.join("src/main.rs");

    println!("cargo:rerun-if-changed={}", observer_manifest.display());
    println!("cargo:rerun-if-changed={}", observer_lock.display());
    println!("cargo:rerun-if-changed={}", observer_main.display());
    println!("cargo:rerun-if-env-changed=ZIG");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("missing OUT_DIR for build script"));
    let linker_path = out_dir.join("zig-linux-musl-linker.sh");
    let zig = env::var("ZIG").unwrap_or_else(|_| "zig".to_string());
    fs::write(
        &linker_path,
        format!(
            "#!/bin/sh\nexec {zig} cc -target x86_64-linux-musl \"$@\"\n",
            zig = shell_quote(&zig)
        ),
    )
    .expect("failed to write zig linker wrapper");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(&linker_path)
            .expect("failed to stat zig linker wrapper")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&linker_path, permissions)
            .expect("failed to mark zig linker wrapper executable");
    }

    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let target_dir = workspace_root.join("target/workspace-observer-host-build");
    let encoded_rustflags = append_encoded_rustflags(
        env::var_os("CARGO_ENCODED_RUSTFLAGS"),
        "-Clink-self-contained=no",
    );
    let status = Command::new(cargo)
        .current_dir(workspace_root)
        .arg("build")
        .arg("--manifest-path")
        .arg(&observer_manifest)
        .arg("--target")
        .arg(OBSERVER_TARGET)
        .arg("--locked")
        .arg("--release")
        .env("CARGO_TARGET_DIR", &target_dir)
        .env(
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER",
            &linker_path,
        )
        .env("CARGO_ENCODED_RUSTFLAGS", encoded_rustflags)
        .status()
        .expect("failed to invoke cargo for workspace observer");
    if !status.success() {
        panic!("failed to build workspace observer for {OBSERVER_TARGET}");
    }

    let built_binary = target_dir
        .join(OBSERVER_TARGET)
        .join("release/workspace-observer");
    if !built_binary.is_file() {
        panic!(
            "workspace observer binary was not produced at {}",
            built_binary.display()
        );
    }

    let packaged_binary = out_dir.join(OBSERVER_OUTPUT_NAME);
    fs::copy(&built_binary, &packaged_binary).unwrap_or_else(|error| {
        panic!(
            "failed to copy workspace observer binary from {} to {}: {error}",
            built_binary.display(),
            packaged_binary.display()
        )
    });
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn append_encoded_rustflags(existing: Option<OsString>, flag: &str) -> OsString {
    let mut rustflags = existing.unwrap_or_default();
    if !rustflags.is_empty() {
        rustflags.push("\u{1f}");
    }
    rustflags.push(flag);
    rustflags
}
