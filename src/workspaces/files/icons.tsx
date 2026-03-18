import type { LucideIcon } from "lucide-react";
import {
	Braces,
	File,
	FileText,
	Folder,
	FolderOpen,
	Image,
	Package,
	Terminal,
} from "lucide-react";

const TEXT_EXTENSIONS = new Set(["md", "mdx", "txt", "rst", "log"]);

const CODE_EXTENSIONS = new Set([
	"c",
	"cc",
	"cpp",
	"cs",
	"css",
	"go",
	"h",
	"html",
	"java",
	"js",
	"json",
	"jsx",
	"php",
	"py",
	"rb",
	"rs",
	"scss",
	"sh",
	"sql",
	"svg",
	"toml",
	"ts",
	"tsx",
	"xml",
	"yaml",
	"yml",
]);

const IMAGE_EXTENSIONS = new Set(["avif", "gif", "jpeg", "jpg", "png", "webp"]);

const PACKAGE_FILENAMES = new Set([
	"bun.lock",
	"bun.lockb",
	"cargo.lock",
	"composer.lock",
	"package-lock.json",
	"package.json",
	"pnpm-lock.yaml",
	"yarn.lock",
]);

const TERMINAL_FILENAMES = new Set([
	".bashrc",
	".env",
	".env.example",
	".gitignore",
	".zshrc",
	"dockerfile",
	"makefile",
]);

export function fileIconForPath(path: string): LucideIcon {
	const name = path.split("/").slice(-1)[0]?.toLowerCase() ?? "";
	const extension = name.includes(".")
		? (name.split(".").slice(-1)[0] ?? "")
		: "";

	if (PACKAGE_FILENAMES.has(name)) {
		return Package;
	}

	if (TERMINAL_FILENAMES.has(name) || extension === "sh") {
		return Terminal;
	}

	if (IMAGE_EXTENSIONS.has(extension)) {
		return Image;
	}

	if (CODE_EXTENSIONS.has(extension)) {
		return Braces;
	}

	if (TEXT_EXTENSIONS.has(extension)) {
		return FileText;
	}

	return File;
}

export const FolderClosedIcon = Folder;
export const FolderOpenIcon = FolderOpen;
