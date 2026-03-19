# AGENTS.md

Silo is a desktop app for working with cloud-hosted development workspaces. It gives the user a local UI for managing projects, terminals, browser tabs, and workspace lifecycle, while each workspace itself runs on a remote VM.

At a high level:

- the frontend is a Vite app
- the desktop shell and backend are Tauri + Rust
- cloud workspaces are remote VMs managed through `gcloud`
- a small Rust daemon, `workspace-agent`, runs inside each workspace VM and reports VM-side state back to the app

Keep this file durable. Prefer guidance on how to discover the current state of the system over documenting specific metadata keys, VM names, or other implementation details that are likely to change.

## Fast checks

- Frontend typecheck: `bun x tsc --noEmit`
- Frontend app dev server: `bun run dev`
- Tauri app: `bun run tauri dev`
- Tauri Rust check: `cd src-tauri && cargo check`
- Tauri Rust tests: `cd src-tauri && cargo test --lib`
- Agent tests: `cd tools/workspace-agent && cargo test`
- Live e2e preflight: `bun run e2e:preflight`
- Live e2e smoke: `bun run e2e`

Use Bun for JS/TS tasks in this repo.

## Where To Look First

When something is broken, identify which layer owns the behavior before changing code:

1. frontend UI state
2. Tauri command/backend state
3. workspace VM state
4. cloud control plane state

Many bugs that look like frontend issues are actually stale VM state, agent state, or metadata drift.

## Local Logs

- By default, Tauri/plugin logs live under `~/.silo/logs`
- When `SILO_STATE_DIR` is set, app-local logs live under `$SILO_STATE_DIR/logs`
- Start by listing recent log files: `ls -lt ~/.silo/logs | head`
- Tail the newest log: `tail -n 200 ~/.silo/logs/<file>`
- If the UI is behaving oddly, also inspect the terminal that launched `bun run tauri dev`

## Local State

Silo stores app-local state under `.silo`. For normal development this is usually `~/.silo`.

- Use `SILO_STATE_DIR` when you need isolated app state without changing the real user home directory
- Prefer this for e2e runs so logs, browser profiles, config, and generated service account keys do not mix with your normal local state
- Keep real `HOME`-backed credentials and tools intact for live-service tests unless the task explicitly requires a separate auth context

## Testing Approach

Prefer a small number of high-value tests over a large mocked suite.

- Default to unit and integration tests for deterministic logic in Rust and TypeScript
- Add e2e tests only for workflows where confidence depends on the full desktop app and real external systems
- Prefer one real journey that crosses the frontend, Tauri backend, and cloud control plane over many narrow UI-only tests
- Avoid broad mock layers for `gcloud`, `gh`, or workspace lifecycle unless the task is specifically about failure injection or an unavailable dependency
- Keep live e2e tests serial and minimal; they are expensive and operationally sensitive

## Live E2E

The current e2e direction is Playwright attached to the CEF runtime over CDP, not WebDriver.

- Use Playwright for desktop e2e against the real CEF webview content
- Enable CDP with `SILO_CEF_REMOTE_DEBUGGING_PORT`
- Keep app-local state isolated with `SILO_STATE_DIR`
- Run `bun run e2e:preflight` before live e2e to verify local tools, auth, and source state
- On macOS, close any existing `Silo.app` instance before running live e2e; the app is single-instance and a second launch can be redirected into the existing process
- When a live test fails, inspect the per-run artifacts under `test-results/e2e/` first, then check the isolated state directory logs for that run

When adding new live e2e coverage:

1. Prefer a real user journey that validates an externally observable result
2. Verify important side effects with the real CLI or control plane, not only through frontend state
3. Add cleanup for every created cloud or GitHub resource in the test harness
4. Keep the suite small enough that engineers will actually run it

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
- inspect the agent state files under `/home/silo/.silo/`
- inspect agent logs with `journalctl`
- verify the agent service is actually running the latest deployed binary

If you change agent code, rebuild and redeploy it before concluding the fix works.

## Architecture Guidance

The app spans local and remote state. Be explicit about which side owns what.

- Prefer backend-owned state over frontend-only merges when correctness matters
- Prefer checking live remote state before adding compatibility or fallback logic
- Avoid documenting transient implementation details here unless they are stable and architectural

## General Guidance

- Prefer discovery over assumption
- Prefer narrow, inspectable fixes over broad compatibility layers
- When a bug may be operational, verify the live system before refactoring code
