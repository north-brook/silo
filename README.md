# Tauri + Next.js

This app uses Tauri v2 with a Next.js frontend configured for static export.

## Development

Run the desktop app in development mode:

```bash
bun run tauri dev
```

Build the frontend static export:

```bash
bun run build
```

## Shared Base Image

The shared workspace base image is built manually by developers and is not part of the Tauri app runtime. The implementation lives in `tools/base-image` and is intended to publish a versioned image into a stable image family that user projects can reference through `gcloud.image_project` and `gcloud.image_family`.

The app defaults now point at the shared family in project `silo-489618`:

```toml
[gcloud]
image_family = "silo-base"
image_project = "silo-489618"
```

Example dry run:

```bash
cd tools/base-image
cargo run -- \
  --project silo-images \
  --family silo-base \
  --member group:silo-users@example.com \
  --dry-run
```

Example publish:

```bash
cd tools/base-image
cargo run -- \
  --project silo-images \
  --family silo-base \
  --member group:silo-users@example.com
```

The binary will:

- create a temporary Ubuntu 24.04 builder VM
- provision the shared toolchain, including Rust
- create a new versioned image in the target family
- grant `roles/compute.imageUser` on the image to each configured IAM member
- delete the builder VM unless `--keep-builder-on-failure` is set

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
