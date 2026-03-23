use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use time::{format_description::parse, OffsetDateTime};

const DEFAULT_IMAGE_FAMILY: &str = "silo-base";
const DEFAULT_SOURCE_IMAGE_FAMILY: &str = "ubuntu-2404-lts-amd64";
const DEFAULT_SOURCE_IMAGE_PROJECT: &str = "ubuntu-os-cloud";
const DEFAULT_ZONE: &str = "us-east4-c";
const DEFAULT_MACHINE_TYPE: &str = "e2-standard-4";
const DEFAULT_DISK_SIZE_GB: u32 = 80;
const DEFAULT_BUILD_TIMEOUT_SECS: u64 = 45 * 60;
const DEFAULT_POLL_INTERVAL_SECS: u64 = 10;
const DEFAULT_PUBLIC_IMAGE_MEMBER: &str = "allAuthenticatedUsers";
const SUCCESS_MARKER: &str = "SILO_BASE_IMAGE_PROVISIONING_COMPLETE";
const FAILURE_MARKER: &str = "SILO_BASE_IMAGE_PROVISIONING_FAILED";
const PROVISION_SCRIPT: &str = include_str!("../scripts/base-image-provision.sh");

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let config = BaseImageConfig::parse(env::args().skip(1).collect())?;
    if config.help {
        print_help();
        return Ok(());
    }

    println!("base image host project: {}", config.project);
    println!("base image family: {}", config.family);
    println!(
        "source image: {}/{}",
        config.source_image_project, config.source_image_family
    );
    println!("builder zone: {}", config.zone);
    println!("builder machine type: {}", config.machine_type);
    println!("share members: {}", render_members(&config.members));
    println!("dry run: {}", config.dry_run);

    let run_id = timestamp_id()?;
    let builder_name = format!("{}-{run_id}", config.builder_name_prefix);
    let image_name = format!("{}-{run_id}", config.family);
    let startup_script_path = write_startup_script()?;

    let result = run_workflow(&config, &builder_name, &image_name, &startup_script_path);

    if startup_script_path.exists() {
        let _ = fs::remove_file(&startup_script_path);
    }

    result
}

fn run_workflow(
    config: &BaseImageConfig,
    builder_name: &str,
    image_name: &str,
    startup_script_path: &Path,
) -> Result<(), String> {
    println!("builder instance: {builder_name}");
    println!("target image: {image_name}");

    let create_args = create_builder_instance_args(
        config,
        builder_name,
        startup_script_path.to_string_lossy().as_ref(),
    );
    print_command(&config.project, config.account.as_deref(), &create_args);
    if config.dry_run {
        print_command(
            &config.project,
            config.account.as_deref(),
            &create_image_args(config, builder_name, image_name),
        );
        print_share_commands(config, image_name);
        return Ok(());
    }

    let mut builder_created = false;
    let workflow_result = (|| -> Result<(), String> {
        run_gcloud(&config.project, config.account.as_deref(), &create_args)?;
        builder_created = true;

        wait_for_provisioning(config, builder_name)?;
        run_gcloud(
            &config.project,
            config.account.as_deref(),
            &stop_builder_instance_args(config, builder_name),
        )?;
        run_gcloud(
            &config.project,
            config.account.as_deref(),
            &create_image_args(config, builder_name, image_name),
        )?;

        for member in &config.members {
            let args = share_image_args(image_name, member);
            run_gcloud(&config.project, config.account.as_deref(), &args)?;
        }

        Ok(())
    })();

    if builder_created {
        if workflow_result.is_ok() || !config.keep_builder_on_failure {
            let delete_args = delete_builder_instance_args(config, builder_name);
            if let Err(error) = run_gcloud(&config.project, config.account.as_deref(), &delete_args)
            {
                eprintln!("warning: failed to delete builder instance {builder_name}: {error}");
            }
        } else {
            eprintln!("warning: keeping failed builder instance {builder_name} for inspection");
        }
    }

    workflow_result?;

    println!("published image {} in family {}", image_name, config.family);
    Ok(())
}

fn print_help() {
    println!(
        "\
Usage:
  cargo run -- --project <image-host-project> [options]

Required:
  --project <id>              GCP project that will host the shared image family

Options:
  --account <email>           gcloud account to use
  --family <name>             Image family name (default: {DEFAULT_IMAGE_FAMILY})
  --zone <zone>               Builder instance zone (default: {DEFAULT_ZONE})
  --machine-type <type>       Builder instance machine type (default: {DEFAULT_MACHINE_TYPE})
  --disk-size-gb <gb>         Builder boot disk size in GB (default: {DEFAULT_DISK_SIZE_GB})
  --source-image-family <id>  Source image family (default: {DEFAULT_SOURCE_IMAGE_FAMILY})
  --source-image-project <id> Source image project (default: {DEFAULT_SOURCE_IMAGE_PROJECT})
  --builder-name-prefix <id>  Prefix for the temporary builder VM (default: silo-base-image-builder)
  --member <principal>        Additional IAM member to grant roles/compute.imageUser on the new image
                             Defaults always include {DEFAULT_PUBLIC_IMAGE_MEMBER} for cross-project image use
                             Example: --member group:silo-users@example.com
  --build-timeout-secs <n>    Wait timeout for startup provisioning (default: {DEFAULT_BUILD_TIMEOUT_SECS})
  --poll-interval-secs <n>    Serial console poll interval (default: {DEFAULT_POLL_INTERVAL_SECS})
  --keep-builder-on-failure   Do not delete the builder VM if provisioning fails
  --dry-run                   Print the gcloud commands without creating resources
  --help                      Show this help text
"
    );
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BaseImageConfig {
    project: String,
    account: Option<String>,
    family: String,
    zone: String,
    machine_type: String,
    disk_size_gb: u32,
    source_image_family: String,
    source_image_project: String,
    builder_name_prefix: String,
    members: Vec<String>,
    build_timeout_secs: u64,
    poll_interval_secs: u64,
    keep_builder_on_failure: bool,
    dry_run: bool,
    help: bool,
}

impl BaseImageConfig {
    fn parse(args: Vec<String>) -> Result<Self, String> {
        let mut config = Self {
            project: String::new(),
            account: None,
            family: DEFAULT_IMAGE_FAMILY.to_string(),
            zone: DEFAULT_ZONE.to_string(),
            machine_type: DEFAULT_MACHINE_TYPE.to_string(),
            disk_size_gb: DEFAULT_DISK_SIZE_GB,
            source_image_family: DEFAULT_SOURCE_IMAGE_FAMILY.to_string(),
            source_image_project: DEFAULT_SOURCE_IMAGE_PROJECT.to_string(),
            builder_name_prefix: "silo-base-image-builder".to_string(),
            members: vec![DEFAULT_PUBLIC_IMAGE_MEMBER.to_string()],
            build_timeout_secs: DEFAULT_BUILD_TIMEOUT_SECS,
            poll_interval_secs: DEFAULT_POLL_INTERVAL_SECS,
            keep_builder_on_failure: false,
            dry_run: false,
            help: false,
        };

        let mut index = 0;
        while index < args.len() {
            let arg = &args[index];
            match arg.as_str() {
                "--project" => {
                    index += 1;
                    config.project = required_value(&args, index, "--project")?;
                }
                "--account" => {
                    index += 1;
                    config.account = Some(required_value(&args, index, "--account")?);
                }
                "--family" => {
                    index += 1;
                    config.family = required_value(&args, index, "--family")?;
                }
                "--zone" => {
                    index += 1;
                    config.zone = required_value(&args, index, "--zone")?;
                }
                "--machine-type" => {
                    index += 1;
                    config.machine_type = required_value(&args, index, "--machine-type")?;
                }
                "--disk-size-gb" => {
                    index += 1;
                    let value = required_value(&args, index, "--disk-size-gb")?;
                    config.disk_size_gb = parse_u32(&value, "--disk-size-gb")?;
                }
                "--source-image-family" => {
                    index += 1;
                    config.source_image_family =
                        required_value(&args, index, "--source-image-family")?;
                }
                "--source-image-project" => {
                    index += 1;
                    config.source_image_project =
                        required_value(&args, index, "--source-image-project")?;
                }
                "--builder-name-prefix" => {
                    index += 1;
                    config.builder_name_prefix =
                        required_value(&args, index, "--builder-name-prefix")?;
                }
                "--member" => {
                    index += 1;
                    let value = required_value(&args, index, "--member")?;
                    if !looks_like_member(&value) {
                        return Err(format!(
                            "--member must look like user:alice@example.com or group:team@example.com, got {value}"
                        ));
                    }
                    push_member(&mut config.members, value);
                }
                "--build-timeout-secs" => {
                    index += 1;
                    let value = required_value(&args, index, "--build-timeout-secs")?;
                    config.build_timeout_secs = parse_u64(&value, "--build-timeout-secs")?;
                }
                "--poll-interval-secs" => {
                    index += 1;
                    let value = required_value(&args, index, "--poll-interval-secs")?;
                    config.poll_interval_secs = parse_u64(&value, "--poll-interval-secs")?;
                }
                "--keep-builder-on-failure" => {
                    config.keep_builder_on_failure = true;
                }
                "--dry-run" => {
                    config.dry_run = true;
                }
                "--help" | "-h" => {
                    config.help = true;
                }
                _ => {
                    return Err(format!("unrecognized argument: {arg}"));
                }
            }
            index += 1;
        }

        if !config.help && config.project.trim().is_empty() {
            return Err("--project is required".to_string());
        }
        if config.disk_size_gb == 0 {
            return Err("--disk-size-gb must be greater than zero".to_string());
        }
        if config.build_timeout_secs == 0 {
            return Err("--build-timeout-secs must be greater than zero".to_string());
        }
        if config.poll_interval_secs == 0 {
            return Err("--poll-interval-secs must be greater than zero".to_string());
        }

        Ok(config)
    }
}

fn required_value(args: &[String], index: usize, flag: &str) -> Result<String, String> {
    args.get(index)
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn parse_u32(value: &str, flag: &str) -> Result<u32, String> {
    value
        .parse::<u32>()
        .map_err(|_| format!("{flag} must be a positive integer"))
}

fn parse_u64(value: &str, flag: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("{flag} must be a positive integer"))
}

fn looks_like_member(value: &str) -> bool {
    if matches!(value.trim(), "allUsers" | "allAuthenticatedUsers") {
        return true;
    }
    let Some((kind, principal)) = value.split_once(':') else {
        return false;
    };
    !kind.trim().is_empty() && !principal.trim().is_empty()
}

fn render_members(members: &[String]) -> String {
    if members.is_empty() {
        "none".to_string()
    } else {
        members.join(", ")
    }
}

fn push_member(members: &mut Vec<String>, value: String) {
    if !members.iter().any(|existing| existing == &value) {
        members.push(value);
    }
}

fn timestamp_id() -> Result<String, String> {
    let now = OffsetDateTime::now_utc();
    let format =
        parse("[year][month][day]-[hour][minute][second]").map_err(|error| error.to_string())?;
    now.format(&format).map_err(|error| error.to_string())
}

fn write_startup_script() -> Result<PathBuf, String> {
    let path = env::temp_dir().join(format!(
        "silo-base-image-provision-{}.sh",
        std::process::id()
    ));
    fs::write(&path, PROVISION_SCRIPT)
        .map_err(|error| format!("failed to write startup script {}: {error}", path.display()))?;
    Ok(path)
}

fn create_builder_instance_args(
    config: &BaseImageConfig,
    builder_name: &str,
    startup_script_path: &str,
) -> Vec<String> {
    vec![
        "compute".to_string(),
        "instances".to_string(),
        "create".to_string(),
        builder_name.to_string(),
        format!("--zone={}", config.zone),
        format!("--machine-type={}", config.machine_type),
        format!("--boot-disk-size={}GB", config.disk_size_gb),
        "--metadata=serial-port-enable=TRUE".to_string(),
        format!("--metadata-from-file=startup-script={startup_script_path}"),
        format!("--image-family={}", config.source_image_family),
        format!("--image-project={}", config.source_image_project),
        "--no-service-account".to_string(),
        "--no-scopes".to_string(),
        "--quiet".to_string(),
    ]
}

fn stop_builder_instance_args(config: &BaseImageConfig, builder_name: &str) -> Vec<String> {
    vec![
        "compute".to_string(),
        "instances".to_string(),
        "stop".to_string(),
        builder_name.to_string(),
        format!("--zone={}", config.zone),
        "--quiet".to_string(),
    ]
}

fn create_image_args(
    config: &BaseImageConfig,
    builder_name: &str,
    image_name: &str,
) -> Vec<String> {
    vec![
        "compute".to_string(),
        "images".to_string(),
        "create".to_string(),
        image_name.to_string(),
        format!("--source-disk={builder_name}"),
        format!("--source-disk-zone={}", config.zone),
        format!("--family={}", config.family),
        "--guest-os-features=VIRTIO_SCSI_MULTIQUEUE".to_string(),
        "--quiet".to_string(),
    ]
}

fn delete_builder_instance_args(config: &BaseImageConfig, builder_name: &str) -> Vec<String> {
    vec![
        "compute".to_string(),
        "instances".to_string(),
        "delete".to_string(),
        builder_name.to_string(),
        format!("--zone={}", config.zone),
        "--quiet".to_string(),
    ]
}

fn serial_output_args(config: &BaseImageConfig, builder_name: &str) -> Vec<String> {
    vec![
        "compute".to_string(),
        "instances".to_string(),
        "get-serial-port-output".to_string(),
        builder_name.to_string(),
        format!("--zone={}", config.zone),
        "--port=1".to_string(),
        "--quiet".to_string(),
    ]
}

fn share_image_args(image_name: &str, member: &str) -> Vec<String> {
    vec![
        "compute".to_string(),
        "images".to_string(),
        "add-iam-policy-binding".to_string(),
        image_name.to_string(),
        format!("--member={member}"),
        "--role=roles/compute.imageUser".to_string(),
        "--quiet".to_string(),
    ]
}

fn run_gcloud(project: &str, account: Option<&str>, args: &[String]) -> Result<String, String> {
    print_command(project, account, args);

    let mut command = Command::new("gcloud");
    if let Some(account) = account.filter(|value| !value.trim().is_empty()) {
        command.arg(format!("--account={account}"));
    }
    if !project.trim().is_empty() {
        command.arg(format!("--project={project}"));
    }

    let output = command
        .args(args)
        .output()
        .map_err(|error| format!("failed to execute gcloud: {error}"))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            Err(format!("gcloud exited with status {}", output.status))
        } else {
            Err(stderr)
        }
    }
}

fn wait_for_provisioning(config: &BaseImageConfig, builder_name: &str) -> Result<(), String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(config.build_timeout_secs);
    let args = serial_output_args(config, builder_name);

    while std::time::Instant::now() < deadline {
        let output = run_gcloud(&config.project, config.account.as_deref(), &args)?;
        if output_contains_marker(&output, SUCCESS_MARKER) {
            println!("startup provisioning completed");
            return Ok(());
        }
        if let Some(code) = failure_code_from_output(&output) {
            return Err(format!("startup provisioning failed with exit code {code}"));
        }

        thread::sleep(Duration::from_secs(config.poll_interval_secs));
    }

    Err(format!(
        "timed out waiting for builder provisioning after {} seconds",
        config.build_timeout_secs
    ))
}

fn output_contains_marker(output: &str, marker: &str) -> bool {
    output.lines().any(|line| line.trim() == marker)
}

fn failure_code_from_output(output: &str) -> Option<i32> {
    output.lines().find_map(|line| {
        let trimmed = line.trim();
        trimmed
            .strip_prefix(&format!("{FAILURE_MARKER}:"))
            .and_then(|value| value.parse::<i32>().ok())
    })
}

fn print_command(project: &str, account: Option<&str>, args: &[String]) {
    let mut rendered = vec!["gcloud".to_string()];
    if let Some(account) = account.filter(|value| !value.trim().is_empty()) {
        rendered.push(format!("--account={account}"));
    }
    if !project.trim().is_empty() {
        rendered.push(format!("--project={project}"));
    }
    rendered.extend(args.iter().cloned());
    println!("$ {}", rendered.join(" "));
}

fn print_share_commands(config: &BaseImageConfig, image_name: &str) {
    for member in &config.members {
        let args = share_image_args(image_name, member);
        print_command(&config.project, config.account.as_deref(), &args);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        failure_code_from_output, output_contains_marker, BaseImageConfig, DEFAULT_IMAGE_FAMILY,
        PROVISION_SCRIPT,
    };

    #[test]
    fn parse_requires_project_unless_help() {
        let error = BaseImageConfig::parse(vec!["--family".to_string(), "custom".to_string()])
            .expect_err("project should be required");
        assert!(error.contains("--project"));

        let config = BaseImageConfig::parse(vec!["--help".to_string()]).expect("help should parse");
        assert!(config.help);
    }

    #[test]
    fn parse_collects_members_and_defaults() {
        let config = BaseImageConfig::parse(vec![
            "--project".to_string(),
            "silo-images".to_string(),
            "--member".to_string(),
            "group:silo@example.com".to_string(),
            "--member".to_string(),
            "user:alice@example.com".to_string(),
        ])
        .expect("config should parse");

        assert_eq!(config.project, "silo-images");
        assert_eq!(config.family, DEFAULT_IMAGE_FAMILY);
        assert_eq!(
            config.members,
            vec![
                "allAuthenticatedUsers".to_string(),
                "group:silo@example.com".to_string(),
                "user:alice@example.com".to_string()
            ]
        );
    }

    #[test]
    fn parse_defaults_to_public_image_access() {
        let config =
            BaseImageConfig::parse(vec!["--project".to_string(), "silo-images".to_string()])
                .expect("config should parse");

        assert_eq!(config.members, vec!["allAuthenticatedUsers".to_string()]);
    }

    #[test]
    fn parse_deduplicates_all_authenticated_users_member() {
        let config = BaseImageConfig::parse(vec![
            "--project".to_string(),
            "silo-images".to_string(),
            "--member".to_string(),
            "allAuthenticatedUsers".to_string(),
        ])
        .expect("config should parse");

        assert_eq!(config.members, vec!["allAuthenticatedUsers".to_string()]);
    }

    #[test]
    fn detects_provisioning_markers() {
        assert!(output_contains_marker(
            "booting\nSILO_BASE_IMAGE_PROVISIONING_COMPLETE\n",
            "SILO_BASE_IMAGE_PROVISIONING_COMPLETE"
        ));
        assert_eq!(
            failure_code_from_output("log\nSILO_BASE_IMAGE_PROVISIONING_FAILED:17\n"),
            Some(17)
        );
        assert_eq!(failure_code_from_output("all good"), None);
    }

    #[test]
    fn provision_script_installs_and_initializes_git_lfs() {
        assert!(PROVISION_SCRIPT.contains("git-lfs"));
        assert!(PROVISION_SCRIPT.contains("git lfs install --system"));
        assert!(PROVISION_SCRIPT.contains("git lfs version"));
    }
}
