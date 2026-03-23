import { Check } from "lucide-react";
import type { ReactNode } from "react";
import { GCloudIcon } from "@/shared/ui/icons/gcloud";
import { Wrench } from "lucide-react";
import { SiloIcon } from "@/shared/ui/icons/silo";
import { Loader } from "@/shared/ui/loader";
import type { WorkspaceLifecycle } from "@/workspaces/api";

function StepRow({
  label,
  active,
  done,
  icon,
}: {
  label: string;
  active: boolean;
  done: boolean;
  icon: ReactNode;
}) {
  return (
    <div className="flex items-center gap-2.5 text-sm">
      <span
        className={`w-3 flex items-center justify-center shrink-0 ${!active && !done ? "opacity-30" : ""}`}
      >
        {icon}
      </span>
      <span
        className={
          done
            ? "text-text-muted"
            : active
              ? "text-text"
              : "text-text-placeholder"
        }
      >
        {label}
      </span>
      <span className="ml-auto w-3 flex items-center justify-center shrink-0">
        {done ? (
          <Check size={10} className="text-green-500" />
        ) : active ? (
          <Loader />
        ) : null}
      </span>
    </div>
  );
}

export function WorkspaceUpdatingScreen({
  lifecycle,
}: {
  lifecycle: WorkspaceLifecycle;
}) {
  return (
    <div className="flex-1 flex flex-col items-center justify-center p-6 gap-4">
      <div className="flex flex-col items-center gap-5">
        <SiloIcon height={36} />
        <div className="flex flex-col gap-1.5 min-w-64">
          <StepRow
            label="Connecting to workspace"
            icon={<GCloudIcon height={12} />}
            active={false}
            done={true}
          />
          <StepRow
            label="Updating workspace observer"
            icon={<Wrench size={12} />}
            active={true}
            done={false}
          />
        </div>
      </div>
      <p className="max-w-md text-center text-sm text-text-muted">
        {lifecycle.detail ??
          "Installing the latest workspace observer before opening the workspace."}
      </p>
      {lifecycle.last_error ? (
        <p className="max-w-md text-center text-sm text-error">
          {lifecycle.last_error}
        </p>
      ) : null}
    </div>
  );
}
