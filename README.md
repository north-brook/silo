# Silo

Silo is a desktop app for working with cloud-hosted development workspaces.

The app gives the user a local UI for:

- managing projects
- creating and resuming remote workspaces
- opening terminals, browser tabs, and files inside those workspaces
- tracking workspace activity and lifecycle state

At a high level:

- the frontend is a Vite + React app in [`src`](./src)
- the website is a standalone Next.js app in [`website`](./website)
- the desktop shell and backend are Tauri + Rust in [`src-tauri`](./src-tauri)
- cloud workspaces are remote VMs managed through `gcloud`
- a small Rust daemon, [`workspace-agent`](./tools/workspace-agent), runs inside each workspace VM
- a repo-local [`driver`](./driver) CLI drives the live desktop app over Playwright + CDP for verification and automation

## Architecture

Silo spans local and remote state, so most behavior crosses multiple layers:

1. the local React UI renders project and workspace state
2. Tauri commands in the Rust backend manage config, process orchestration, and cloud operations
3. remote workspace VMs host terminals, browser/file attachments, and the workspace agent
4. `gcloud` is the control plane for workspace lifecycle and metadata

The main backend areas in [`src-tauri/src`](./src-tauri/src) are:

- [`projects.rs`](./src-tauri/src/projects.rs) for local project config and discovery
- [`workspaces.rs`](./src-tauri/src/workspaces.rs) for workspace lifecycle and metadata
- [`remote.rs`](./src-tauri/src/remote.rs) for remote command execution over `gcloud compute ssh`
- [`terminal.rs`](./src-tauri/src/terminal.rs), [`browser.rs`](./src-tauri/src/browser.rs), and [`files.rs`](./src-tauri/src/files.rs) for workspace attachments

The main frontend areas in [`src`](./src) are:

- [`src/app`](./src/app) for app bootstrap, routing, and the main shell
- [`src/dashboard`](./src/dashboard) for the home/dashboard screen
- [`src/projects`](./src/projects) for project APIs and sidebar flows
- [`src/workspaces`](./src/workspaces) for workspace state, routes, and layout

## Workspace Agent

[`tools/workspace-agent`](./tools/workspace-agent) is deployed inside each remote workspace VM as `/home/silo/.silo/bin/workspace-agent`.

It is responsible for:

- publishing workspace activity and heartbeat metadata back to the VM
- observing terminal and assistant session state
- tracking unread and working status for remote sessions
- managing a small runtime state directory under `/home/silo/.silo/workspace-agent`
- exposing file tree/read/write and file watch commands for workspace file attachments

Key pieces:

- [`src/cli.rs`](./tools/workspace-agent/src/cli.rs) defines the command surface
- [`src/daemon/mod.rs`](./tools/workspace-agent/src/daemon/mod.rs) runs the long-lived observer/publisher loop
- [`src/daemon/state.rs`](./tools/workspace-agent/src/daemon/state.rs) models published session and file-watch state
- [`src/assistant.rs`](./tools/workspace-agent/src/assistant.rs) wraps assistant commands and emits activity events
- [`src/runtime.rs`](./tools/workspace-agent/src/runtime.rs) manages the FIFO, pidfile, and persisted runtime state

## Driver

[`driver`](./driver) is a repo-local automation layer for launching and driving the real desktop app over Playwright + CDP.

Use it for:

- live app inspection
- scripted verification
- coding-agent automation
- trace collection with stable JSON output

Typical flow:

```bash
bun run driver -- session launch
bun run driver -- app status --session <id>
bun run driver -- page snapshot --session <id>
bun run driver -- session close --session <id>
```

Useful commands:

- `bun run driver -- help`
- `bun run driver -- schema`
- `bun run driver -- history`
- `bun run driver -- batch --session <id> --file ./steps.json`
- `bun run driver -- session list`
- `bun run driver -- app service-status --session <id>`
- `bun run driver -- element click --session <id> --selector testid:...`

More detail lives in [`driver/README.md`](./driver/README.md).

## Development

Use Bun for JS/TS tasks in this repo.

Install dependencies:

```bash
bun install
```

Run the frontend dev server:

```bash
bun run dev
```

Run the website:

```bash
bun run website:dev
```

Run the desktop app:

```bash
bun run tauri dev
```

`bun run tauri dev` launches the isolated development app identity (`Silo Dev`) and uses `~/.silo-dev` unless `SILO_STATE_DIR` is set. Production builds keep the `Silo` app identity and default to `~/.silo`.

Build the frontend:

```bash
bun run build
```

Build the website:

```bash
bun run website:build
```

Check frontend types:

```bash
bun x tsc --noEmit
```

Check the Rust backend:

```bash
cd src-tauri && cargo check
```

## Testing

Frontend and repo checks:

```bash
bun run check
```

Rust library tests:

```bash
cd src-tauri && cargo test --lib
```

Workspace agent tests:

```bash
cd tools/workspace-agent && cargo test
```

Live e2e preflight:

```bash
bun run e2e:preflight
```

Live e2e smoke:

```bash
bun run e2e
```

The e2e suite lives in [`tests/e2e`](./tests/e2e) and currently targets the live desktop app over Playwright.

## Production Releases

Pushes to `main` trigger [`.github/workflows/release.yml`](./.github/workflows/release.yml), which builds the production macOS app, publishes a GitHub Release, and uploads the updater artifacts and `latest.json`.

Production clients do not talk to GitHub directly:

- the app checks `https://silo.new/update`
- fresh installs use `https://silo.new/download`
- the `silo.new` website routes resolve the current GitHub Release assets and redirect to the exact installer or updater bundle

The website supports these optional server-side environment variables so the backing GitHub repository or installer asset can change without shipping a new desktop build:

- `SILO_RELEASE_GITHUB_REPOSITORY` defaults to `north-brook/silo`
- `SILO_RELEASE_INSTALLER_ASSET_NAME` defaults to `Silo-macos-arm64.dmg`

Update those environment variables on the `silo.new` deployment and redeploy the website when the backing release repository or installer asset name changes.

Required GitHub secrets:

- `SILO_UPDATER_PUBLIC_KEY`
- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
- `APPLE_CERTIFICATE`
- `APPLE_CERTIFICATE_PASSWORD`
- `APPLE_SIGNING_IDENTITY`
- `APPLE_ID`
- `APPLE_PASSWORD`
- `APPLE_TEAM_ID`

## Repo Layout

- [`src`](./src): frontend app
- [`src-tauri`](./src-tauri): desktop shell and backend
- [`driver`](./driver): live desktop automation CLI
- [`tests/e2e`](./tests/e2e): live Playwright coverage
- [`scripts`](./scripts): repo utilities and e2e preflight entrypoints
- [`tools/workspace-agent`](./tools/workspace-agent): remote VM agent
- [`tools/base-image`](./tools/base-image): shared base image publisher for workspaces

## Shared Base Image

The shared workspace base image is built manually by developers and is not part of the Tauri app runtime. The implementation lives in [`tools/base-image`](./tools/base-image) and publishes a versioned image into a stable image family that user projects can reference through `gcloud.image_project` and `gcloud.image_family`.

Example config:

```toml
[gcloud]
image_family = "silo-base"
image_project = "silo-489618"
```

Dry run:

```bash
cd tools/base-image
cargo run -- \
  --project silo-images \
  --family silo-base \
  --dry-run
```

Publish:

```bash
cd tools/base-image
cargo run -- \
  --project silo-images \
  --family silo-base
```

The tool will:

- create a temporary Ubuntu 24.04 builder VM
- provision the shared toolchain, including Rust and Docker Engine
- create a new versioned image in the target family
- grant `roles/compute.imageUser` on the image to `allAuthenticatedUsers` plus any additional configured IAM members
- delete the builder VM unless `--keep-builder-on-failure` is set

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/)
- [Tauri VS Code extension](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode)
- [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
