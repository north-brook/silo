# AGENTS.md

Silo is a desktop app for working with cloud-hosted development workspaces. It gives the user a local UI for managing projects, terminals, browser tabs, and workspace lifecycle, while each workspace itself runs on a remote VM.

At a high level:

- the frontend is a Next.js app
- the desktop shell and backend are Tauri + Rust
- cloud workspaces are remote VMs managed through `gcloud`
- a small Rust daemon, `workspace-observer`, runs inside each workspace VM and reports VM-side state back to the app

Keep this file durable. Prefer guidance on how to discover the current state of the system over documenting specific metadata keys, VM names, or other implementation details that are likely to change.

## Fast checks

- Frontend typecheck: `bun x tsc --noEmit`
- Frontend app dev server: `bun run dev`
- Tauri app: `bun run tauri dev`
- Tauri Rust check: `cd src-tauri && cargo check`
- Tauri Rust tests: `cd src-tauri && cargo test --lib`
- Observer tests: `cd tools/workspace-observer && cargo test`

Use Bun for JS/TS tasks in this repo.

## Where To Look First

When something is broken, identify which layer owns the behavior before changing code:

1. frontend UI state
2. Tauri command/backend state
3. workspace VM state
4. cloud control plane state

Many bugs that look like frontend issues are actually stale VM state, observer state, or metadata drift.

## Local Logs

- Tauri/plugin logs live under `~/.silo/logs`
- Start by listing recent log files: `ls -lt ~/.silo/logs | head`
- Tail the newest log: `tail -n 200 ~/.silo/logs/<file>`
- If the UI is behaving oddly, also inspect the terminal that launched `bun run tauri dev`

## Cloud VMs

Do not hardcode VM names. Discover them from `gcloud`.

- List instances: `gcloud compute instances list`
- Filter to running instances when needed
- Include zone, status, and name in output so you can target the right VM

Once you know the instance and zone:

- Interactive shell: `gcloud compute ssh <instance> --zone=<zone>`
- One-off command: `gcloud compute ssh <instance> --zone=<zone> --command='<command>'`

Useful first commands on a VM:

- `uptime`
- `free -h`
- `df -h`
- `ps -eo pid,pcpu,pmem,args --sort=-pcpu | head -n 20`

For performance issues, inspect live CPU and memory usage before assuming the cause.

## VM Metadata And Observer State

If behavior depends on remote workspace state, inspect the live VM rather than assuming the code and metadata agree.

Useful techniques:

- read instance metadata from the metadata server on the VM
- inspect the observer state files under `/home/silo/.silo/`
- inspect observer logs with `journalctl`
- verify the observer service is actually running the latest deployed binary

If you change observer code, rebuild and redeploy it before concluding the fix works.

## Architecture Guidance

The app spans local and remote state. Be explicit about which side owns what.

- Prefer backend-owned state over frontend-only merges when correctness matters
- Prefer checking live remote state before adding compatibility or fallback logic
- Avoid documenting transient implementation details here unless they are stable and architectural

## General Guidance

- Prefer discovery over assumption
- Prefer narrow, inspectable fixes over broad compatibility layers
- When a bug may be operational, verify the live system before refactoring code
