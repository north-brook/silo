import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import ts from "typescript";

const ROOT = process.cwd();
const CLASS_HELPERS = new Set(["classNames", "classnames", "clsx", "cn"]);
const SCRIPT_EXTENSIONS = new Set([".js", ".jsx", ".ts", ".tsx"]);
const STYLE_EXTENSIONS = new Set([".css"]);
const SKIP_DIRECTORIES = new Set([
	".git",
	".next",
	"dist",
	"node_modules",
	"out",
	"target",
]);

type Violation = {
	file: string;
	line: number;
	column: number;
	className: string;
	token: string;
};

function walkFiles(dir: string, files: string[] = []) {
	for (const entry of readdirSync(dir, { withFileTypes: true })) {
		if (entry.isDirectory()) {
			if (SKIP_DIRECTORIES.has(entry.name)) {
				continue;
			}
			walkFiles(path.join(dir, entry.name), files);
			continue;
		}

		const extension = path.extname(entry.name);
		if (!SCRIPT_EXTENSIONS.has(extension) && !STYLE_EXTENSIONS.has(extension)) {
			continue;
		}

		files.push(path.join(dir, entry.name));
	}

	return files;
}

function collectCustomCssClasses(files: string[]) {
	const classes = new Set<string>();
	const selectorPattern =
		/(^|[^\w-])\.([A-Za-z_-][A-Za-z0-9_-]*)(?=[\s.{:#>[,+~[])/gm;

	for (const file of files) {
		if (!STYLE_EXTENSIONS.has(path.extname(file))) {
			continue;
		}

		const text = readFileSync(file, "utf8");
		for (const match of text.matchAll(selectorPattern)) {
			const className = match[2];
			if (className) {
				classes.add(className);
			}
		}
	}

	return classes;
}

function isClassHelperCall(expression: ts.Expression): boolean {
	if (ts.isIdentifier(expression)) {
		return CLASS_HELPERS.has(expression.text);
	}

	if (ts.isPropertyAccessExpression(expression)) {
		return CLASS_HELPERS.has(expression.name.text);
	}

	return false;
}

function isClassNameContext(node: ts.Node): boolean {
	for (
		let current: ts.Node | undefined = node;
		current;
		current = current.parent
	) {
		if (
			ts.isJsxAttribute(current) &&
			ts.isIdentifier(current.name) &&
			current.name.text === "className"
		) {
			return true;
		}

		if (ts.isCallExpression(current) && isClassHelperCall(current.expression)) {
			return true;
		}
	}

	return false;
}

function collectLiteralTexts(node: ts.Node) {
	if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) {
		return [{ text: node.text, start: node.getStart() + 1 }];
	}

	if (ts.isTemplateExpression(node)) {
		const segments = [
			{ text: node.head.text, start: node.head.getStart() + 1 },
		];
		for (const span of node.templateSpans) {
			segments.push({
				text: span.literal.text,
				start: span.literal.getStart() + 1,
			});
		}
		return segments;
	}

	return [];
}

function collectViolations(file: string, customClasses: Set<string>) {
	const sourceText = readFileSync(file, "utf8");
	const sourceFile = ts.createSourceFile(
		file,
		sourceText,
		ts.ScriptTarget.Latest,
		true,
		file.endsWith(".tsx")
			? ts.ScriptKind.TSX
			: file.endsWith(".ts")
				? ts.ScriptKind.TS
				: file.endsWith(".jsx")
					? ts.ScriptKind.JSX
					: ts.ScriptKind.JS,
	);
	const violations: Violation[] = [];

	function visit(node: ts.Node) {
		if (isClassNameContext(node)) {
			for (const { text, start } of collectLiteralTexts(node)) {
				for (const token of text.split(/\s+/).filter(Boolean)) {
					if (!token.includes(":")) {
						continue;
					}

					const className = token.slice(token.lastIndexOf(":") + 1);
					if (!customClasses.has(className)) {
						continue;
					}

					const tokenOffset = text.indexOf(token);
					const position = sourceFile.getLineAndCharacterOfPosition(
						start + Math.max(tokenOffset, 0),
					);

					violations.push({
						file,
						line: position.line + 1,
						column: position.character + 1,
						className,
						token,
					});
				}
			}
		}

		ts.forEachChild(node, visit);
	}

	visit(sourceFile);
	return violations;
}

const files = walkFiles(ROOT);
const customClasses = collectCustomCssClasses(files);
const sourceFiles = files.filter((file) =>
	SCRIPT_EXTENSIONS.has(path.extname(file)),
);
const violations = sourceFiles.flatMap((file) =>
	collectViolations(file, customClasses),
);

if (violations.length === 0) {
	console.log(
		`tailwind-custom-variants: checked ${sourceFiles.length} source files, no variant-prefixed custom CSS classes found`,
	);
	process.exit(0);
}

for (const violation of violations) {
	const relativePath = path.relative(ROOT, violation.file);
	console.error(
		`${relativePath}:${violation.line}:${violation.column} variant-prefixed custom CSS class "${violation.token}"`,
	);
	console.error(
		`  "${violation.className}" is defined in local CSS. Do not use Tailwind variant syntax with custom classes; write an explicit CSS selector instead.`,
	);
}

process.exit(1);
