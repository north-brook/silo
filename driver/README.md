# Driver

`driver/` is a repo-local automation layer for launching and driving the Silo desktop app over Playwright + CDP.

It is intended for:

- programmatic verification
- coding-agent automation
- manual debugging with stable JSON output

Typical flow:

```sh
bun run driver -- session launch
bun run driver -- app status --session <id>
bun run driver -- app service-status --session <id> --service gcloud
bun run driver -- element assert-text --session <id> --selector testid:setup-status-gcloud --contains connected
bun run driver -- page snapshot --session <id>
bun run driver -- session close --session <id>
```

`session launch` already waits for app readiness before returning.

Core selector forms:

- `testid:dashboard-action-open-project`
- `testid:setup-status-gcloud`
- `text:Open Project`
- `label:Back`
- `role:button[name="Open Project"]`
- `css:.some-class`

Useful commands:

- `help`, `help <command>`
- `schema`
- `history`
- `batch`
- `session list`, `session status`, `session close`
- `video status`
- `page snapshot`, `page console`, `page network`
- `element text`, `element html`, `element attr`, `element exists`, `element count`
- `element assert`, `element assert-text`, `element assert-attr`
- `app wait-ready`, `app status`, `app service-status`

All commands print JSON to stdout and use nonzero exit codes on failure. Human-readable help and error details are printed to stderr.

Commands that attach to a running app now require an explicit session target via `--session <id>` or `SILO_DRIVER_SESSION`. Use `latest` only when you intentionally want the newest session.

Driver runs also emit trace bundles under the active Silo home, typically `~/.silo/traces/<trace-id>/`.
Each trace directory includes:

- `manifest.json`
- `driver.jsonl`
- `app.log`
- `video.mp4`
- `video-metadata.json`
- `tauri.stdout.log`
- `tauri.stderr.log`
- `video.stdout.log`
- `video.stderr.log`
- `vite.stdout.log` / `vite.stderr.log` when the driver started Vite itself

Global driver CLI history is written to `~/.silo/traces/driver-history.jsonl`.

Video capture starts automatically for driver-launched sessions and finalizes into `video.mp4` when the session closes.
Use `bun run driver -- video status --session <id>` for a live session or `bun run driver -- video status --trace-id <trace-id>` after close.

For agents or scripted workflows, prefer `bun run driver -- schema` for command metadata and `bun run driver -- batch --session <id>` to run multiple steps over one CDP connection.
