#!/usr/bin/env python3

from __future__ import annotations

import json
import os
import subprocess
import sys
import tomllib
from pathlib import Path


def resolved_account(global_gcloud: dict, project_gcloud: dict) -> str:
    service_account = str(global_gcloud.get("service_account", "")).strip()
    if service_account:
        return service_account

    override = str(project_gcloud.get("account", "")).strip()
    if override:
        return override

    return str(global_gcloud.get("account", "")).strip()


def resolved_project(global_gcloud: dict, project_gcloud: dict) -> str:
    override = str(project_gcloud.get("project", "")).strip()
    if override:
        return override

    return str(global_gcloud.get("project", "")).strip()


def load_gcloud_targets(config_path: Path) -> list[tuple[str, str]]:
    with config_path.open("rb") as config_file:
        config = tomllib.load(config_file)

    global_gcloud = config.get("gcloud", {})
    projects = config.get("projects", {})

    targets: list[tuple[str, str]] = []
    seen: set[tuple[str, str]] = set()

    for project_config in projects.values():
        project_gcloud = project_config.get("gcloud", {})
        account = resolved_account(global_gcloud, project_gcloud)
        project = resolved_project(global_gcloud, project_gcloud)
        if not account or not project:
            continue
        key = (account, project)
        if key not in seen:
            seen.add(key)
            targets.append(key)

    global_account = str(
        global_gcloud.get("service_account") or global_gcloud.get("account", "")
    ).strip()
    global_project = str(global_gcloud.get("project", "")).strip()
    if global_account and global_project:
        key = (global_account, global_project)
        if key not in seen:
            targets.append(key)

    return targets


def run_gcloud_json(account: str, project: str, args: list[str]) -> list[dict]:
    command = [
        "gcloud",
        f"--account={account}",
        f"--project={project}",
        *args,
        "--format=json",
    ]
    result = subprocess.run(command, check=True, capture_output=True, text=True)
    payload = result.stdout.strip()
    if not payload:
        return []
    value = json.loads(payload)
    if isinstance(value, list):
        return value
    return [value]


def run_gcloud(account: str, project: str, args: list[str]) -> None:
    command = [
        "gcloud",
        f"--account={account}",
        f"--project={project}",
        *args,
    ]
    subprocess.run(command, check=True)


def list_silo_instances(account: str, project: str) -> list[tuple[str, str]]:
    instances = run_gcloud_json(
        account,
        project,
        [
            "compute",
            "instances",
            "list",
            "--filter=name~'.*-silo-.*'",
        ],
    )
    results: list[tuple[str, str]] = []
    for instance in instances:
        name = instance.get("name")
        zone = instance.get("zone", "")
        if not isinstance(name, str) or not isinstance(zone, str):
            continue
        zone_name = zone.rsplit("/", 1)[-1]
        if zone_name:
            results.append((name, zone_name))
    return results


def list_template_snapshots(account: str, project: str) -> list[str]:
    snapshots = run_gcloud_json(
        account,
        project,
        [
            "compute",
            "snapshots",
            "list",
            "--filter=labels.template=true",
        ],
    )
    names: list[str] = []
    for snapshot in snapshots:
        name = snapshot.get("name")
        if isinstance(name, str) and name:
            names.append(name)
    return names


def main() -> int:
    dry_run = "--dry-run" in sys.argv[1:]
    config_path = Path(os.path.expanduser("~/.silo/config.toml"))
    if not config_path.exists():
        print(f"config not found: {config_path}", file=sys.stderr)
        return 1

    targets = load_gcloud_targets(config_path)
    if not targets:
        print("no configured gcloud targets found")
        return 0

    for account, project in targets:
        print(f"[{project}] account={account}")

        instances = list_silo_instances(account, project)
        if instances:
            for name, zone in instances:
                print(f"delete instance {name} ({zone})")
                if not dry_run:
                    run_gcloud(
                        account,
                        project,
                        [
                            "compute",
                            "instances",
                            "delete",
                            name,
                            f"--zone={zone}",
                            "--quiet",
                        ],
                    )
        else:
            print("no silo instances found")

        snapshots = list_template_snapshots(account, project)
        if snapshots:
            for name in snapshots:
                print(f"delete snapshot {name}")
                if not dry_run:
                    run_gcloud(
                        account,
                        project,
                        [
                            "compute",
                            "snapshots",
                            "delete",
                            name,
                            "--quiet",
                        ],
                    )
        else:
            print("no template snapshots found")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
