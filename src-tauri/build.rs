use base64::Engine as _;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const OBSERVER_TARGET: &str = "x86_64-unknown-linux-musl";
const AGENT_OUTPUT_NAME: &str = "workspace-agent-x86_64-unknown-linux-musl";
const BUILD_FLAVOR_ENV_VAR: &str = "SILO_BUILD_FLAVOR";
const GITHUB_REPOSITORY_ENV_VAR: &str = "SILO_GITHUB_REPOSITORY";
const UPDATER_ENDPOINT_ENV_VAR: &str = "SILO_UPDATER_ENDPOINT";
const UPDATER_PUBLIC_KEY_ENV_VAR: &str = "SILO_UPDATER_PUBLIC_KEY";

fn main() {
    emit_build_metadata();
    build_workspace_agent();
    tauri_build::build()
}

fn emit_build_metadata() {
    println!("cargo:rerun-if-env-changed={BUILD_FLAVOR_ENV_VAR}");
    println!("cargo:rerun-if-env-changed={GITHUB_REPOSITORY_ENV_VAR}");
    println!("cargo:rerun-if-env-changed={UPDATER_ENDPOINT_ENV_VAR}");
    println!("cargo:rerun-if-env-changed={UPDATER_PUBLIC_KEY_ENV_VAR}");

    let build_flavor = env::var(BUILD_FLAVOR_ENV_VAR).unwrap_or_else(|_| "dev".to_string());
    println!("cargo:rustc-env={BUILD_FLAVOR_ENV_VAR}={build_flavor}");

    if let Ok(repository) = env::var(GITHUB_REPOSITORY_ENV_VAR) {
        println!("cargo:rustc-env={GITHUB_REPOSITORY_ENV_VAR}={repository}");
    }

    if let Ok(endpoint) = env::var(UPDATER_ENDPOINT_ENV_VAR) {
        println!("cargo:rustc-env={UPDATER_ENDPOINT_ENV_VAR}={endpoint}");
    }

    let updater_public_key = env::var(UPDATER_PUBLIC_KEY_ENV_VAR)
        .ok()
        .and_then(|value| normalize_updater_public_key_for_runtime(&value));

    if build_flavor == "prod" && updater_public_key.is_none() {
        panic!("{UPDATER_PUBLIC_KEY_ENV_VAR} must be set when building the production app");
    }

    if let Some(public_key) = updater_public_key {
        println!(
            "cargo:rustc-env={UPDATER_PUBLIC_KEY_ENV_VAR}={}",
            encode_rustc_env_value(&public_key)
        );
    }
}

fn normalize_updater_public_key_for_runtime(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.contains("untrusted comment:") {
        return Some(encode_canonical_updater_public_key(trimmed));
    }

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(trimmed)
        .ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let decoded = decoded.trim();
    if decoded.contains("untrusted comment:") {
        return Some(encode_canonical_updater_public_key(decoded));
    }

    None
}

fn encode_canonical_updater_public_key(value: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(format!("{value}\n"))
}

fn encode_rustc_env_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\n', "\\n")
}

fn build_workspace_agent() {
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .expect("src-tauri should live under the workspace root");
    let agent_dir = workspace_root.join("tools/workspace-agent");
    let agent_manifest = agent_dir.join("Cargo.toml");
    let agent_lock = agent_dir.join("Cargo.lock");
    let agent_main = agent_dir.join("src/main.rs");

    println!("cargo:rerun-if-changed={}", agent_manifest.display());
    println!("cargo:rerun-if-changed={}", agent_lock.display());
    println!("cargo:rerun-if-changed={}", agent_main.display());
    println!("cargo:rerun-if-env-changed=ZIG");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("missing OUT_DIR for build script"));
    let linker_path = out_dir.join("zig-linux-musl-linker.sh");
    let zig = env::var("ZIG").unwrap_or_else(|_| "zig".to_string());
    fs::write(
        &linker_path,
        format!(
            "#!/usr/bin/env bash\n\
args=()\n\
while (($#)); do\n\
  case \"$1\" in\n\
    --target=*)\n\
      shift\n\
      ;;\n\
    --target)\n\
      if (($# >= 2)); then\n\
        shift 2\n\
      else\n\
        shift\n\
      fi\n\
      ;;\n\
    *)\n\
      args+=(\"$1\")\n\
      shift\n\
      ;;\n\
  esac\n\
done\n\
exec {zig} cc -target x86_64-linux-musl \"${{args[@]}}\"\n",
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
    let target_dir = workspace_root.join("target/workspace-agent-host-build");
    let encoded_rustflags = append_encoded_rustflags(
        env::var_os("CARGO_ENCODED_RUSTFLAGS"),
        "-Clink-self-contained=no",
    );
    let status = Command::new(cargo)
        .current_dir(workspace_root)
        .arg("build")
        .arg("--manifest-path")
        .arg(&agent_manifest)
        .arg("--target")
        .arg(OBSERVER_TARGET)
        .arg("--locked")
        .arg("--release")
        .env("CARGO_TARGET_DIR", &target_dir)
        .env(
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER",
            &linker_path,
        )
        .env("CC_x86_64_unknown_linux_musl", &linker_path)
        .env("CC_x86_64-unknown-linux-musl", &linker_path)
        .env("CARGO_ENCODED_RUSTFLAGS", encoded_rustflags)
        .status()
        .expect("failed to invoke cargo for workspace agent");
    if !status.success() {
        panic!("failed to build workspace agent for {OBSERVER_TARGET}");
    }

    let built_binary = target_dir
        .join(OBSERVER_TARGET)
        .join("release/workspace-agent");
    if !built_binary.is_file() {
        panic!(
            "workspace agent binary was not produced at {}",
            built_binary.display()
        );
    }

    let packaged_binary = out_dir.join(AGENT_OUTPUT_NAME);
    fs::copy(&built_binary, &packaged_binary).unwrap_or_else(|error| {
        panic!(
            "failed to copy workspace agent binary from {} to {}: {error}",
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
