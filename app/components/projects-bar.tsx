import { convertFileSrc } from "@tauri-apps/api/core";
import { useProjects, type ListedProject } from "../hooks/use-projects";

function ProjectRow({ project }: { project: ListedProject }) {
	return (
		<button
			type="button"
			className="flex items-center gap-2.5 w-full px-3 py-2 text-xs text-text hover:bg-btn-hover hover:text-text-bright transition-colors"
		>
			{project.image ? (
				<img
					src={convertFileSrc(project.image)}
					alt={project.name}
					className="w-5 h-5 rounded object-cover shrink-0"
				/>
			) : (
				<div className="w-5 h-5 rounded bg-border-light shrink-0" />
			)}
			<span className="truncate">{project.name}</span>
		</button>
	);
}

export function ProjectsBar() {
	const projects = useProjects();
	const hasProjects = projects.data && projects.data.length > 0;

	if (!hasProjects) return null;

	return (
		<aside className="fixed top-0 left-0 bottom-6 w-48 border-r border-border-light bg-surface flex flex-col">
			<div className="flex-1 overflow-y-auto py-1">
				{projects.data.map((project) => (
					<ProjectRow key={project.name} project={project} />
				))}
			</div>
		</aside>
	);
}
