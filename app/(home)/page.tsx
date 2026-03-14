"use client";

import { useQuery } from "@tanstack/react-query";
import { FolderOpen, Plus } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { invoke } from "../../lib/invoke";
import type { ListedProject } from "../../lib/projects";
import { useNewWorkspace } from "../components/new-workspace";
import { useOpenProject } from "../components/open-project";
import { StatusIcons } from "../components/status-icons";
import { SiloIcon } from "../icons/silo";

function Kbd({ children }: { children: React.ReactNode }) {
	return (
		<kbd className="inline-flex items-center justify-center w-5 h-5 rounded border border-border-light bg-btn text-[11px] text-text-muted">
			{children}
		</kbd>
	);
}

function ActionRow({
	icon: Icon,
	label,
	keys,
	onClick,
	disabled,
}: {
	icon: LucideIcon;
	label: string;
	keys: React.ReactNode[];
	onClick: () => void;
	disabled?: boolean;
}) {
	return (
		<button
			type="button"
			onClick={onClick}
			disabled={disabled}
			className="flex items-center gap-3 w-full px-1 py-2.5 text-sm text-text-muted hover:text-text-bright transition-colors disabled:opacity-50 cursor-pointer"
		>
			<Icon size={16} className="shrink-0" />
			<span className="flex-1 text-left">{label}</span>
			<span className="shrink-0 flex items-center gap-1">
				{keys.map((key, i) => (
					// biome-ignore lint/suspicious/noArrayIndexKey: static list
					<Kbd key={i}>{key}</Kbd>
				))}
			</span>
		</button>
	);
}

export default function HomePage() {
	const newWorkspace = useNewWorkspace();
	const openProject = useOpenProject();
	const projects = useQuery({
		queryKey: ["projects_list_projects"],
		queryFn: () => invoke<ListedProject[]>("projects_list_projects"),
	});
	const hasProjects = (projects.data ?? []).length > 0;

	return (
		<>
			<div data-tauri-drag-region className="h-8 shrink-0" />
			<div className="flex flex-col items-center justify-center flex-1">
				<SiloIcon height={32} />
				<div className="flex flex-col mt-10 w-64">
					<ActionRow
						icon={FolderOpen}
						label="Open Project"
						keys={["\u21E7", "\u2318", "O"]}
						onClick={() => openProject.open()}
						disabled={openProject.isPending}
					/>
					{hasProjects && (
						<ActionRow
							icon={Plus}
							label="New Workspace"
							keys={["\u2318", "N"]}
							onClick={() => newWorkspace.open()}
						/>
					)}
				</div>
			</div>
			<div className="shrink-0 flex items-center justify-between px-3 py-2">
				<span className="text-[11px] text-text-muted">v0.1.0</span>
				<StatusIcons />
			</div>
		</>
	);
}
