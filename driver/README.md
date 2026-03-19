# Driver

`driver/` is a repo-local automation layer for launching and driving the Silo desktop app over Playwright + CDP.

It is intended for:

- programmatic verification
- coding-agent automation
- manual debugging with stable JSON output

Typical flow:

```sh
bun run driver -- launch
bun run driver -- app.wait-ready
bun run driver -- app.status
bun run driver -- app.service-status --service gcloud
bun run driver -- assert-text --selector testid:setup-status-gcloud --contains connected
bun run driver -- snapshot
bun run driver -- close
```

Core selector forms:

- `testid:dashboard-action-open-project`
- `testid:setup-status-gcloud`
- `text:Open Project`
- `label:Back`
- `role:button[name="Open Project"]`
- `css:.some-class`

Useful commands:

- `help`, `help <command>`
- `history`
- `video.status`
- `sessions.list`
- `tabs.list`
- `text`, `html`, `attr`, `exists`, `count`
- `assert`, `assert-text`, `assert-attr`
- `app.wait-ready`, `app.status`, `app.service-status`

All commands print JSON to stdout and use nonzero exit codes on failure. Human-readable help and error details are printed to stderr.

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
Use `bun run driver -- video.status --session <id>` for a live session or `bun run driver -- video.status --trace-id <trace-id>` after close.
