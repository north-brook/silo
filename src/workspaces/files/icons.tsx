import type { LucideIcon } from "lucide-react";
import {
	Braces,
	Cog,
	Database,
	File,
	FileCode2,
	FileText,
	Folder,
	FolderOpen,
	Globe,
	Image,
	Lock,
	Package,
	Palette,
	Terminal,
} from "lucide-react";

const LOCK_FILENAMES = new Set([
	"bun.lock",
	"bun.lockb",
	"cargo.lock",
	"composer.lock",
	"package-lock.json",
	"pnpm-lock.yaml",
	"yarn.lock",
]);

const PACKAGE_FILENAMES = new Set([
	"cargo.toml",
	"gemfile",
	"go.mod",
	"go.sum",
	"package.json",
	"pyproject.toml",
	"requirements.txt",
]);

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
	"tsconfig.json",
]);

const TERMINAL_FILENAMES = new Set([
	".bashrc",
	".zshrc",
	"dockerfile",
	"makefile",
]);

const IMAGE_EXTENSIONS = new Set([
	"avif",
	"gif",
	"ico",
	"jpeg",
	"jpg",
	"png",
	"svg",
	"webp",
]);

const CSS_EXTENSIONS = new Set(["css", "less", "sass", "scss"]);
const HTML_EXTENSIONS = new Set(["htm", "html"]);
const JSON_EXTENSIONS = new Set(["json", "json5", "jsonc"]);
const CONFIG_EXTENSIONS = new Set(["cfg", "ini", "toml", "yaml", "yml"]);
const TEXT_EXTENSIONS = new Set(["log", "md", "mdx", "rst", "txt"]);

const CODE_EXTENSIONS = new Set([
	"c",
	"cc",
	"cjs",
	"cpp",
	"cs",
	"go",
	"h",
	"hpp",
	"java",
	"js",
	"jsx",
	"kt",
	"lua",
	"mjs",
	"php",
	"py",
	"rb",
	"rs",
	"svelte",
	"swift",
	"ts",
	"tsx",
	"vue",
	"xml",
	"zig",
]);

export function fileIconForPath(path: string): LucideIcon {
	const name = path.split("/").slice(-1)[0]?.toLowerCase() ?? "";
	const extension = name.includes(".")
		? (name.split(".").slice(-1)[0] ?? "")
		: "";

	// Exact filename matches
	if (LOCK_FILENAMES.has(name)) return Lock;
	if (PACKAGE_FILENAMES.has(name)) return Package;
	if (CONFIG_FILENAMES.has(name)) return Cog;
	if (TERMINAL_FILENAMES.has(name)) return Terminal;

	// Config patterns: .env*, *.config.ts/js, tsconfig.*.json
	if (name.startsWith(".env")) return Cog;
	if (/\.config\.(ts|js|mjs|cjs)$/.test(name)) return Cog;
	if (name.startsWith("tsconfig") && extension === "json") return Cog;

	// Shell scripts before code extensions
	if (extension === "sh" || extension === "bash" || extension === "zsh")
		return Terminal;

	// Extension-based matches
	if (HTML_EXTENSIONS.has(extension)) return Globe;
	if (CSS_EXTENSIONS.has(extension)) return Palette;
	if (JSON_EXTENSIONS.has(extension)) return Braces;
	if (CONFIG_EXTENSIONS.has(extension)) return Cog;
	if (extension === "sql") return Database;
	if (IMAGE_EXTENSIONS.has(extension)) return Image;
	if (TEXT_EXTENSIONS.has(extension)) return FileText;
	if (CODE_EXTENSIONS.has(extension)) return FileCode2;

	return File;
}

export const FolderClosedIcon = Folder;
export const FolderOpenIcon = FolderOpen;
