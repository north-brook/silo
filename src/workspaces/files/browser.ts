const BROWSER_RENDERABLE_EXTENSIONS = new Set([
	"gif",
	"jpeg",
	"jpg",
	"pdf",
	"png",
	"svg",
	"webp",
]);

export function filePathOpensInBrowser(path: string): boolean {
	const trimmed = path.trim();
	if (!trimmed) {
		return false;
	}

	const fileName = trimmed.split("/").pop() ?? trimmed;
	const extension = fileName.includes(".")
		? (fileName.split(".").pop() ?? "").toLowerCase()
		: "";
	return BROWSER_RENDERABLE_EXTENSIONS.has(extension);
}
