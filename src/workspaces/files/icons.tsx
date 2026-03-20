const LOCK_FILENAMES = new Set([
	"bun.lock",
	"bun.lockb",
	"cargo.lock",
	"composer.lock",
	"package-lock.json",
	"pnpm-lock.yaml",
	"yarn.lock",
]);

const PACKAGE_FILENAMES: Record<string, string> = {
	"cargo.toml": "rust",
	"go.mod": "go",
	"go.sum": "go",
	"package.json": "npm",
	"pyproject.toml": "python",
	"requirements.txt": "python",
	gemfile: "ruby",
};

const CONFIG_FILENAMES = new Set([
	".editorconfig",
	".eslintrc",
	".eslintrc.json",
	".eslintrc.js",
	".gitattributes",
	".gitignore",
	".npmrc",
	".nvmrc",
	".prettierrc",
	".prettierrc.json",
	".prettierignore",
]);

const TERMINAL_FILENAMES = new Set([".bashrc", ".zshrc", "makefile"]);

const ICON_URLS: Record<string, string> = {};

function getIconUrl(name: string): string {
	if (!ICON_URLS[name]) {
		ICON_URLS[name] = new URL(`./icons/${name}.svg`, import.meta.url).href;
	}
	return ICON_URLS[name];
}

function getIconName(path: string): string {
	const name = path.split("/").pop()?.toLowerCase() ?? "";
	const ext = name.includes(".") ? (name.split(".").pop() ?? "") : "";

	// Exact filename matches
	if (LOCK_FILENAMES.has(name)) return "lock";
	if (name in PACKAGE_FILENAMES) return PACKAGE_FILENAMES[name];
	if (CONFIG_FILENAMES.has(name)) return "settings";
	if (TERMINAL_FILENAMES.has(name)) return "shell";
	if (name === "dockerfile" || name.startsWith("dockerfile."))
		return "docker";
	if (name === "tsconfig.json" || name.startsWith("tsconfig."))
		return "typescript";

	// Config patterns
	if (name.startsWith(".env")) return "settings";
	if (/\.config\.(ts|js|mjs|cjs)$/.test(name)) return "settings";

	// Extension matches
	switch (ext) {
		case "ts":
			return "typescript";
		case "tsx":
			return "react-ts";
		case "js":
		case "mjs":
		case "cjs":
			return "javascript";
		case "jsx":
			return "react-jsx";
		case "json":
		case "json5":
		case "jsonc":
			return "json";
		case "html":
		case "htm":
			return "html";
		case "css":
		case "less":
			return "css";
		case "scss":
		case "sass":
			return "sass";
		case "py":
			return "python";
		case "rs":
			return "rust";
		case "go":
			return "go";
		case "md":
		case "mdx":
			return "markdown";
		case "yaml":
		case "yml":
			return "yaml";
		case "toml":
		case "ini":
		case "cfg":
			return "toml";
		case "sh":
		case "bash":
		case "zsh":
			return "shell";
		case "sql":
			return "sql";
		case "xml":
			return "xml";
		case "svg":
			return "svg";
		case "png":
		case "jpg":
		case "jpeg":
		case "gif":
		case "webp":
		case "avif":
		case "ico":
			return "image";
		case "rb":
			return "ruby";
		case "java":
			return "java";
		case "c":
			return "c";
		case "cc":
		case "cpp":
		case "cxx":
			return "cpp";
		case "h":
		case "hpp":
			return "h";
		case "cs":
			return "csharp";
		case "swift":
			return "swift";
		case "kt":
			return "kotlin";
		case "php":
			return "php";
		case "vue":
			return "vue";
		case "svelte":
			return "svelte";
		case "lua":
			return "lua";
		case "zig":
			return "zig";
		case "txt":
		case "log":
		case "rst":
			return "text";
		default:
			return "document";
	}
}

export function FileIcon({
	path,
	size = 12,
	className,
}: {
	path: string;
	size?: number;
	className?: string;
}) {
	const iconName = getIconName(path);
	return (
		<img
			src={getIconUrl(iconName)}
			width={size}
			height={size}
			alt=""
			draggable={false}
			className={className}
		/>
	);
}
