import { appendFileSync, existsSync, readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import type { BrowserContext, Locator, Page } from "@playwright/test";
import {
	Command,
	CommanderError,
	InvalidArgumentError,
	Option,
} from "commander";
import { readAppServiceStatuses, readAppStatus, waitForAppReady } from "./app";
import {
	defaultSourceStateDir,
	traceDirFor,
	traceHistoryLogPath,
} from "./paths";
import { isPidRunning } from "./processes";
import {
	connectToDriverSession,
	DriverLaunchError,
	disconnectFromDriverSession,
	launchDriverSession,
	stopLaunchedSession,
} from "./runtime";
import { parseSelector, resolveLocator } from "./selectors";
import {
	listSessionRecords,
	removeSessionRecord,
	resolveSessionRecord,
	writeSessionRecord,
} from "./session-store";
import type { ConsoleEntry, DriverSessionRecord } from "./types";
import { ensureDirectory } from "./utils";

type CommandResult = Record<string, unknown>;

type DriverCommandLogEntry = {
	argv: string[];
	command: string;
	durationMs: number;
	error: string | null;
	flags: Record<string, unknown>;
	ok: boolean;
	pid: number;
	startedAt: string;
};

type ConnectedCommandTarget = {
	context: BrowserContext;
	page: Page;
	session: DriverSessionRecord;
};

type SessionOptions = {
	session?: string;
};

type TimeoutOptions = {
	timeout?: number;
};

type BatchStep = {
	command: string;
	[key: string]: unknown;
};

let currentUserArgv: string[] = [];

class CliError extends Error {
	command?: string;
	hint?: string;
	usage?: string[];

	constructor(
		message: string,
		options: { command?: string; hint?: string; usage?: string[] } = {},
	) {
		super(message);
		this.name = "CliError";
		this.command = options.command;
		this.hint = options.hint;
		this.usage = options.usage;
	}
}

function parseIntegerOption(name: string) {
	return (value: string) => {
		const parsed = Number.parseInt(value, 10);
		if (!Number.isFinite(parsed)) {
			throw new InvalidArgumentError(
				`Expected an integer for --${name}, got "${value}".`,
			);
		}
		return parsed;
	};
}

function sessionOption() {
	return new Option(
		"--session <id>",
		"Driver session id. Use `latest` to target the newest session explicitly.",
	).env("SILO_DRIVER_SESSION");
}

function sourceStateDirOption() {
	return new Option(
		"--source-state-dir <path>",
		"Source Silo state dir. Defaults to the active user state dir.",
	);
}

function timeoutOption() {
	return new Option("--timeout <ms>", "Timeout in milliseconds.").argParser(
		parseIntegerOption("timeout"),
	);
}

function dottedCommandPath(command: Command) {
	const segments: string[] = [];
	let current: Command | null = command;
	while (current?.parent) {
		segments.unshift(current.name());
		current = current.parent;
	}
	return segments.join(".");
}

function findSubcommand(parent: Command, name: string) {
	return parent.commands.find(
		(command) => command.name() === name || command.aliases().includes(name),
	);
}

function resolveCommandByPath(program: Command, pathTokens: string[]) {
	let current = program;
	for (const token of pathTokens) {
		const next = findSubcommand(current, token);
		if (!next) {
			throw new CliError(`Unknown command: ${pathTokens.join(".")}`, {
				hint: "Run `bun run driver -- help` to list available commands.",
			});
		}
		current = next;
	}
	return current;
}

function commandSummary(command: Command) {
	return command.summary() || command.description() || null;
}

function commandJson(command: Command) {
	return {
		command: dottedCommandPath(command) || "driver",
		description: command.description() || null,
		options: command.options.map((option) => ({
			defaultValue: option.defaultValue ?? null,
			description: option.description || null,
			flags: option.flags,
			name: option.attributeName(),
			required: option.mandatory ?? false,
		})),
		subcommands: command.commands.map((child) => ({
			command: dottedCommandPath(child),
			summary: commandSummary(child),
		})),
		summary: commandSummary(command),
		usage: command.usage(),
	};
}

function normalizeFlags(
	options: Record<string, unknown> | undefined,
): Record<string, unknown> {
	if (!options) {
		return {};
	}

	return Object.fromEntries(
		Object.entries(options).filter(([, value]) => value !== undefined),
	);
}

function globalDriverCommandLogPath(sourceStateDir: string) {
	return traceHistoryLogPath(sourceStateDir);
}

function writeCommandLogEntry(
	commandLogPath: string,
	entry: DriverCommandLogEntry,
) {
	ensureDirectory(path.dirname(commandLogPath));
	appendFileSync(commandLogPath, `${JSON.stringify(entry)}\n`);
}

function sessionFromResult(result: CommandResult) {
	const candidate = result.session;
	if (!candidate || typeof candidate !== "object") {
		return undefined;
	}

	return candidate as DriverSessionRecord;
}

function tryResolveLoggingSession(
	commandName: string,
	flags: Record<string, unknown>,
) {
	const selection =
		typeof flags.session === "string" ? flags.session : undefined;
	if (!selection) {
		return undefined;
	}

	try {
		return resolveSessionSelection(commandName, selection);
	} catch {
		return undefined;
	}
}

function logCommandAttempt(
	commandName: string,
	flags: Record<string, unknown>,
	startedAt: string,
	startedAtMs: number,
	options: {
		error: string | null;
		result?: CommandResult;
		session?: DriverSessionRecord;
	},
) {
	const session = options.result
		? (sessionFromResult(options.result) ?? options.session)
		: options.session;
	const sourceStateDir =
		session?.sourceStateDir ??
		(typeof flags.sourceStateDir === "string"
			? flags.sourceStateDir
			: defaultSourceStateDir);
	const entry: DriverCommandLogEntry = {
		argv: currentUserArgv,
		command: commandName,
		durationMs: Date.now() - startedAtMs,
		error: options.error,
		flags,
		ok: options.error === null,
		pid: process.pid,
		startedAt,
	};

	writeCommandLogEntry(globalDriverCommandLogPath(sourceStateDir), entry);
	if (session?.driverLogPath) {
		writeCommandLogEntry(session.driverLogPath, entry);
	}
}

function printJson(value: unknown) {
	process.stdout.write(
		`${JSON.stringify({ ok: true, ...((value as object) ?? {}) }, null, 2)}\n`,
	);
}

function printError(error: unknown) {
	const cliError =
		error instanceof CliError
			? error
			: new CliError(error instanceof Error ? error.message : String(error));
	if (cliError.command) {
		process.stderr.write(
			`${cliError.command}: run \`bun run driver -- help ${cliError.command}\` for usage.\n`,
		);
	}
	process.stderr.write(`${cliError.message}\n`);
	if (cliError.hint) {
		process.stderr.write(`${cliError.hint}\n`);
	}
	process.stdout.write(
		`${JSON.stringify(
			{
				ok: false,
				error: cliError.message,
				hint: cliError.hint ?? null,
				command: cliError.command ?? null,
				usage: cliError.usage ?? null,
			},
			null,
			2,
		)}\n`,
	);
}

function printCommanderError(error: CommanderError) {
	process.stdout.write(
		`${JSON.stringify(
			{
				ok: false,
				error: error.message,
				hint: null,
				command: null,
				usage: null,
			},
			null,
			2,
		)}\n`,
	);
}

function resolveSessionSelection(commandName: string, selection: string) {
	try {
		return resolveSessionRecord(selection);
	} catch (error) {
		const message = error instanceof Error ? error.message : String(error);
		if (message.includes("No driver sessions found")) {
			throw new CliError(message, {
				command: commandName,
				hint: "Launch a session with `bun run driver -- session launch` first.",
			});
		}

		if (selection === "latest") {
			throw new CliError(message, {
				command: commandName,
				hint: "Launch a session with `bun run driver -- session launch` first.",
			});
		}

		throw new CliError(`Unknown driver session: ${selection}`, {
			command: commandName,
			hint: "Run `bun run driver -- session list` to inspect available sessions.",
		});
	}
}

function resolveRequiredSession(commandName: string, options: SessionOptions) {
	if (!options.session) {
		throw new CliError("Missing required option --session <id>.", {
			command: commandName,
			hint: "Pass `--session <id>` or set `SILO_DRIVER_SESSION`. Use `latest` only when you explicitly want the newest session.",
		});
	}

	return resolveSessionSelection(commandName, options.session);
}

function resolveOptionalSession(commandName: string, options: SessionOptions) {
	if (!options.session) {
		return undefined;
	}

	return resolveSessionSelection(commandName, options.session);
}

async function withConnectedSession<T extends CommandResult>(
	commandName: string,
	options: SessionOptions,
	callback: (target: ConnectedCommandTarget) => Promise<T>,
) {
	const session = resolveRequiredSession(commandName, options);
	let connected: Awaited<ReturnType<typeof connectToDriverSession>>;
	try {
		connected = await connectToDriverSession(session);
	} catch (error) {
		const message = error instanceof Error ? error.message : String(error);
		throw new CliError(
			`Failed to attach to driver session ${session.id} over CDP. ${message}`,
			{
				command: commandName,
				hint: `The session may be stale or the app may have exited. Run \`bun run driver -- session status --session ${session.id}\` or launch a fresh session with \`bun run driver -- session launch\`.`,
			},
		);
	}

	try {
		await connected.page
			.waitForLoadState("domcontentloaded")
			.catch(() => undefined);
		return await callback({
			context: connected.context,
			page: connected.page,
			session,
		});
	} finally {
		await disconnectFromDriverSession(connected);
	}
}

async function firstMatch(locator: Locator, timeout?: number) {
	const first = locator.first();
	await first.waitFor({ state: "attached", timeout });
	return first;
}

async function readLocatorText(locator: Locator, timeout?: number) {
	const first = await firstMatch(locator, timeout);
	return (await first.textContent()) ?? "";
}

function trimText(value: string, raw: boolean) {
	return raw ? value : value.trim();
}

function locatorSummary(selector: string) {
	return {
		parsedSelector: parseSelector(selector),
		selector,
	};
}

function latestSessionLog(session: DriverSessionRecord) {
	if (session.appLogPath && existsSync(session.appLogPath)) {
		return session.appLogPath;
	}

	const logsDir = path.join(session.stateDir, "logs");
	const latest = readdirSync(logsDir)
		.filter((entry) => entry.endsWith(".log"))
		.sort()
		.pop();

	if (!latest) {
		throw new CliError(`No session log files found in ${logsDir}`, {
			command: "page.console",
			hint: "Launch the driver again or inspect the session artifacts directory.",
		});
	}

	return path.join(logsDir, latest);
}

function readConsoleEntries(
	session: DriverSessionRecord,
	levelFilter?: string,
): ConsoleEntry[] {
	const contents = readFileSync(latestSessionLog(session), "utf8");
	const levelOrder = new Map([
		["DEBUG", 10],
		["INFO", 20],
		["WARN", 30],
		["ERROR", 40],
	]);
	const minimumLevel = levelFilter
		? (levelOrder.get(levelFilter.toUpperCase()) ?? 10)
		: 10;

	return contents
		.split("\n")
		.map((line) => line.trim())
		.filter((line) => line.includes("[webview:"))
		.map((line) => {
			const match = line.match(
				/^\[(?<date>[^\]]+)\]\[(?<time>[^\]]+)\]\[(?<level>[^\]]+)\]\[(?<target>[^\]]+)\] (?<message>.*)$/,
			);
			if (!match?.groups) {
				return {
					level: null,
					message: line,
					raw: line,
					target: null,
					timestamp: null,
				} satisfies ConsoleEntry;
			}

			return {
				level: match.groups.level,
				message: match.groups.message,
				raw: line,
				target: match.groups.target,
				timestamp: `${match.groups.date} ${match.groups.time}`,
			} satisfies ConsoleEntry;
		})
		.filter((entry) => {
			if (!entry.level) {
				return true;
			}
			return (levelOrder.get(entry.level.toUpperCase()) ?? 10) >= minimumLevel;
		});
}

async function assertLocatorState(
	locator: Locator,
	state: "attached" | "detached" | "hidden" | "visible",
	timeout?: number,
) {
	await locator.first().waitFor({ state, timeout });
}

async function assertBooleanState(
	check: () => Promise<boolean>,
	description: string,
	timeout: number,
) {
	const started = Date.now();
	while (Date.now() - started < timeout) {
		if (await check()) {
			return;
		}
		await new Promise((resolve) => setTimeout(resolve, 200));
	}

	throw new Error(`Timed out waiting for ${description}.`);
}

async function pageSnapshot(target: ConnectedCommandTarget) {
	return {
		pageUrl: target.page.url(),
		sessionId: target.session.id,
		snapshot: await target.page.locator("body").ariaSnapshot(),
	};
}

async function pageScreenshot(
	target: ConnectedCommandTarget,
	options: { fullPage?: boolean; output?: string },
) {
	const output = options.output
		? path.resolve(options.output)
		: path.join(target.session.artifactsDir, `screenshot-${Date.now()}.png`);
	await target.page.screenshot({
		fullPage: options.fullPage ?? false,
		path: output,
	});
	return { output, sessionId: target.session.id };
}

async function pageNetwork(target: ConnectedCommandTarget) {
	return {
		pageUrl: target.page.url(),
		requests: await target.page.evaluate(() =>
			performance.getEntriesByType("resource").map((entry) => {
				const resource = entry as PerformanceResourceTiming;
				return {
					duration: resource.duration,
					initiatorType: resource.initiatorType,
					name: resource.name,
					startTime: resource.startTime,
					transferSize:
						typeof resource.transferSize === "number"
							? resource.transferSize
							: null,
				};
			}),
		),
		sessionId: target.session.id,
	};
}

async function elementClick(
	target: ConnectedCommandTarget,
	options: { selector: string; timeout?: number },
) {
	const locator = resolveLocator(target.page, options.selector);
	await locator.click({ timeout: options.timeout });
	return {
		action: "click",
		sessionId: target.session.id,
		...locatorSummary(options.selector),
	};
}

async function elementType(
	target: ConnectedCommandTarget,
	options: {
		selector: string;
		slowly?: boolean;
		submit?: boolean;
		text: string;
		timeout?: number;
	},
) {
	const locator = resolveLocator(target.page, options.selector);
	await locator.waitFor({ state: "visible", timeout: options.timeout });
	if (options.slowly) {
		await locator.click({ timeout: options.timeout });
		await locator.pressSequentially(options.text);
	} else {
		await locator.fill(options.text, { timeout: options.timeout });
	}
	if (options.submit) {
		await target.page.keyboard.press("Enter");
	}
	return {
		action: "type",
		length: options.text.length,
		sessionId: target.session.id,
		...locatorSummary(options.selector),
	};
}

async function pagePress(
	target: ConnectedCommandTarget,
	options: { key: string },
) {
	await target.page.keyboard.press(options.key);
	return { action: "press", key: options.key, sessionId: target.session.id };
}

async function elementWait(
	target: ConnectedCommandTarget,
	options: {
		selector: string;
		state?: "attached" | "detached" | "hidden" | "visible";
		timeout?: number;
	},
) {
	const state = options.state ?? "visible";
	await resolveLocator(target.page, options.selector).waitFor({
		state,
		timeout: options.timeout,
	});
	return {
		action: "wait",
		sessionId: target.session.id,
		state,
		...locatorSummary(options.selector),
	};
}

async function pageEval(
	target: ConnectedCommandTarget,
	options: { js: string },
) {
	return {
		result: await target.page.evaluate((expression) => {
			// biome-ignore lint/security/noGlobalEval: driver-only escape hatch
			return globalThis.eval(expression);
		}, options.js),
		sessionId: target.session.id,
	};
}

async function elementText(
	target: ConnectedCommandTarget,
	options: { raw?: boolean; selector: string; timeout?: number },
) {
	const text = await readLocatorText(
		resolveLocator(target.page, options.selector),
		options.timeout,
	);
	return {
		sessionId: target.session.id,
		text: trimText(text, options.raw ?? false),
		...locatorSummary(options.selector),
	};
}

async function elementHtml(
	target: ConnectedCommandTarget,
	options: { selector: string; timeout?: number },
) {
	const first = await firstMatch(
		resolveLocator(target.page, options.selector),
		options.timeout,
	);
	return {
		html: await first.evaluate((element) => element.outerHTML),
		sessionId: target.session.id,
		...locatorSummary(options.selector),
	};
}

async function elementAttr(
	target: ConnectedCommandTarget,
	options: { name: string; selector: string; timeout?: number },
) {
	const first = await firstMatch(
		resolveLocator(target.page, options.selector),
		options.timeout,
	);
	return {
		name: options.name,
		sessionId: target.session.id,
		value: await first.getAttribute(options.name),
		...locatorSummary(options.selector),
	};
}

async function elementExists(
	target: ConnectedCommandTarget,
	options: { selector: string; visible?: boolean },
) {
	const locator = resolveLocator(target.page, options.selector);
	const count = await locator.count();
	const exists = options.visible
		? await locator
				.first()
				.isVisible()
				.catch(() => false)
		: count > 0;
	return {
		count,
		exists,
		sessionId: target.session.id,
		visibleOnly: options.visible ?? false,
		...locatorSummary(options.selector),
	};
}

async function elementCount(
	target: ConnectedCommandTarget,
	options: { selector: string },
) {
	return {
		count: await resolveLocator(target.page, options.selector).count(),
		sessionId: target.session.id,
		...locatorSummary(options.selector),
	};
}

async function elementAssert(
	target: ConnectedCommandTarget,
	options: {
		attached?: boolean;
		detached?: boolean;
		disabled?: boolean;
		enabled?: boolean;
		hidden?: boolean;
		selector: string;
		timeout?: number;
		visible?: boolean;
	},
) {
	const timeout = options.timeout ?? 5_000;
	const locator = resolveLocator(target.page, options.selector);
	const checks = [
		["visible", options.visible],
		["hidden", options.hidden],
		["attached", options.attached],
		["detached", options.detached],
		["enabled", options.enabled],
		["disabled", options.disabled],
	].filter(([, enabled]) => enabled);

	if (checks.length > 1) {
		throw new CliError("Use only one assert state flag at a time.", {
			command: "element.assert",
			hint: "Choose one of --visible, --hidden, --attached, --detached, --enabled, or --disabled.",
		});
	}

	const [assertion] = checks;
	const state = (assertion?.[0] as string | undefined) ?? "visible";
	switch (state) {
		case "visible":
		case "hidden":
		case "attached":
		case "detached":
			await assertLocatorState(
				locator,
				state as "attached" | "detached" | "hidden" | "visible",
				timeout,
			);
			break;
		case "enabled":
			await firstMatch(locator, timeout);
			await assertBooleanState(
				() =>
					locator
						.first()
						.isEnabled()
						.catch(() => false),
				`${options.selector} to become enabled`,
				timeout,
			);
			break;
		case "disabled":
			await firstMatch(locator, timeout);
			await assertBooleanState(
				() =>
					locator
						.first()
						.isDisabled()
						.catch(() => false),
				`${options.selector} to become disabled`,
				timeout,
			);
			break;
		default:
			throw new CliError(`Unsupported assert state: ${state}`, {
				command: "element.assert",
			});
	}

	return {
		asserted: state,
		sessionId: target.session.id,
		...locatorSummary(options.selector),
	};
}

async function elementAssertText(
	target: ConnectedCommandTarget,
	options: {
		contains?: string;
		equals?: string;
		raw?: boolean;
		selector: string;
		timeout?: number;
	},
) {
	if (!options.equals && !options.contains) {
		throw new CliError("Provide either --equals or --contains.", {
			command: "element.assert-text",
			hint: "Example: bun run driver -- element assert-text --selector css:h1 --equals Silo",
		});
	}
	if (options.equals && options.contains) {
		throw new CliError("Use only one of --equals or --contains.", {
			command: "element.assert-text",
		});
	}

	const actual = trimText(
		await readLocatorText(
			resolveLocator(target.page, options.selector),
			options.timeout ?? 5_000,
		),
		options.raw ?? false,
	);

	if (typeof options.equals === "string" && actual !== options.equals) {
		throw new CliError(
			`Text assertion failed for ${options.selector}: expected exactly "${options.equals}", got "${actual}".`,
			{ command: "element.assert-text" },
		);
	}
	if (
		typeof options.contains === "string" &&
		!actual.includes(options.contains)
	) {
		throw new CliError(
			`Text assertion failed for ${options.selector}: expected text containing "${options.contains}", got "${actual}".`,
			{ command: "element.assert-text" },
		);
	}

	return {
		actual,
		asserted: options.equals ? "equals" : "contains",
		expected: options.equals ?? options.contains,
		sessionId: target.session.id,
		...locatorSummary(options.selector),
	};
}

async function elementAssertAttr(
	target: ConnectedCommandTarget,
	options: {
		contains?: string;
		equals?: string;
		exists?: boolean;
		name: string;
		selector: string;
		timeout?: number;
	},
) {
	const checks = [
		Boolean(options.equals),
		Boolean(options.contains),
		Boolean(options.exists),
	].filter(Boolean).length;
	if (checks !== 1) {
		throw new CliError(
			"Choose exactly one attribute assertion: --exists, --equals, or --contains.",
			{ command: "element.assert-attr" },
		);
	}

	const first = await firstMatch(
		resolveLocator(target.page, options.selector),
		options.timeout ?? 5_000,
	);
	const value = await first.getAttribute(options.name);

	if (options.exists && value === null) {
		throw new CliError(
			`Attribute assertion failed for ${options.selector}: expected attribute "${options.name}" to exist.`,
			{ command: "element.assert-attr" },
		);
	}
	if (typeof options.equals === "string" && value !== options.equals) {
		throw new CliError(
			`Attribute assertion failed for ${options.selector}: expected ${options.name}="${options.equals}", got ${value === null ? "null" : `"${value}"`}.`,
			{ command: "element.assert-attr" },
		);
	}
	if (
		typeof options.contains === "string" &&
		!(value ?? "").includes(options.contains)
	) {
		throw new CliError(
			`Attribute assertion failed for ${options.selector}: expected ${options.name} to contain "${options.contains}", got ${value === null ? "null" : `"${value}"`}.`,
			{ command: "element.assert-attr" },
		);
	}

	return {
		actual: value,
		asserted: options.exists
			? "exists"
			: options.equals
				? "equals"
				: "contains",
		expected: options.equals ?? options.contains ?? true,
		name: options.name,
		sessionId: target.session.id,
		...locatorSummary(options.selector),
	};
}

async function appWaitReady(
	target: ConnectedCommandTarget,
	options: TimeoutOptions,
) {
	return {
		sessionId: target.session.id,
		status: await waitForAppReady(target.page, options.timeout ?? 60_000),
	};
}

async function appStatus(target: ConnectedCommandTarget) {
	return {
		sessionId: target.session.id,
		status: await readAppStatus(target.page),
	};
}

async function appServiceStatus(
	target: ConnectedCommandTarget,
	options: { service?: string },
) {
	const services = await readAppServiceStatuses(target.page);
	if (!options.service) {
		return {
			services,
			sessionId: target.session.id,
		};
	}

	if (!(options.service in services)) {
		throw new CliError(`Unknown service: ${options.service}`, {
			command: "app.service-status",
			hint: "Use one of: gcloud, github, codex, claude.",
		});
	}

	return {
		service: options.service,
		sessionId: target.session.id,
		value: services[options.service as keyof typeof services],
	};
}

function normalizeBatchCommandName(command: string) {
	const trimmed = command.trim();
	switch (trimmed) {
		case "page.snapshot":
		case "page.screenshot":
		case "page.network":
		case "page.press":
		case "page.eval":
		case "element.click":
		case "element.type":
		case "element.wait":
		case "element.text":
		case "element.html":
		case "element.attr":
		case "element.exists":
		case "element.count":
		case "element.assert":
		case "element.assert-text":
		case "element.assert-attr":
		case "app.wait-ready":
		case "app.status":
		case "app.service-status":
			return trimmed;
	}

	throw new CliError(`Unsupported batch step command: ${command}`, {
		command: "batch",
		hint: "Use canonical names like `element.click`, `page.snapshot`, `app.status`, or `app.service-status`.",
	});
}

async function runBatchStep(target: ConnectedCommandTarget, step: BatchStep) {
	const command = normalizeBatchCommandName(step.command);
	switch (command) {
		case "page.snapshot":
			return pageSnapshot(target);
		case "page.screenshot":
			return pageScreenshot(target, {
				fullPage: Boolean(step.fullPage),
				output: typeof step.output === "string" ? step.output : undefined,
			});
		case "page.network":
			return pageNetwork(target);
		case "page.press":
			if (typeof step.key !== "string") {
				throw new CliError("Batch step page.press requires `key`.", {
					command: "batch",
				});
			}
			return pagePress(target, { key: step.key });
		case "page.eval":
			if (typeof step.js !== "string") {
				throw new CliError("Batch step page.eval requires `js`.", {
					command: "batch",
				});
			}
			return pageEval(target, { js: step.js });
		case "element.click":
			if (typeof step.selector !== "string") {
				throw new CliError("Batch step element.click requires `selector`.", {
					command: "batch",
				});
			}
			return elementClick(target, {
				selector: step.selector,
				timeout: typeof step.timeout === "number" ? step.timeout : undefined,
			});
		case "element.type":
			if (typeof step.selector !== "string" || typeof step.text !== "string") {
				throw new CliError(
					"Batch step element.type requires `selector` and `text`.",
					{ command: "batch" },
				);
			}
			return elementType(target, {
				selector: step.selector,
				slowly: Boolean(step.slowly),
				submit: Boolean(step.submit),
				text: step.text,
				timeout: typeof step.timeout === "number" ? step.timeout : undefined,
			});
		case "element.wait":
			if (typeof step.selector !== "string") {
				throw new CliError("Batch step element.wait requires `selector`.", {
					command: "batch",
				});
			}
			return elementWait(target, {
				selector: step.selector,
				state:
					typeof step.state === "string"
						? (step.state as "attached" | "detached" | "hidden" | "visible")
						: undefined,
				timeout: typeof step.timeout === "number" ? step.timeout : undefined,
			});
		case "element.text":
			if (typeof step.selector !== "string") {
				throw new CliError("Batch step element.text requires `selector`.", {
					command: "batch",
				});
			}
			return elementText(target, {
				raw: Boolean(step.raw),
				selector: step.selector,
				timeout: typeof step.timeout === "number" ? step.timeout : undefined,
			});
		case "element.html":
			if (typeof step.selector !== "string") {
				throw new CliError("Batch step element.html requires `selector`.", {
					command: "batch",
				});
			}
			return elementHtml(target, {
				selector: step.selector,
				timeout: typeof step.timeout === "number" ? step.timeout : undefined,
			});
		case "element.attr":
			if (typeof step.selector !== "string" || typeof step.name !== "string") {
				throw new CliError(
					"Batch step element.attr requires `selector` and `name`.",
					{ command: "batch" },
				);
			}
			return elementAttr(target, {
				name: step.name,
				selector: step.selector,
				timeout: typeof step.timeout === "number" ? step.timeout : undefined,
			});
		case "element.exists":
			if (typeof step.selector !== "string") {
				throw new CliError("Batch step element.exists requires `selector`.", {
					command: "batch",
				});
			}
			return elementExists(target, {
				selector: step.selector,
				visible: Boolean(step.visible),
			});
		case "element.count":
			if (typeof step.selector !== "string") {
				throw new CliError("Batch step element.count requires `selector`.", {
					command: "batch",
				});
			}
			return elementCount(target, { selector: step.selector });
		case "element.assert":
			if (typeof step.selector !== "string") {
				throw new CliError("Batch step element.assert requires `selector`.", {
					command: "batch",
				});
			}
			return elementAssert(target, {
				attached: Boolean(step.attached),
				detached: Boolean(step.detached),
				disabled: Boolean(step.disabled),
				enabled: Boolean(step.enabled),
				hidden: Boolean(step.hidden),
				selector: step.selector,
				timeout: typeof step.timeout === "number" ? step.timeout : undefined,
				visible: Boolean(step.visible),
			});
		case "element.assert-text":
			if (typeof step.selector !== "string") {
				throw new CliError(
					"Batch step element.assert-text requires `selector`.",
					{ command: "batch" },
				);
			}
			return elementAssertText(target, {
				contains: typeof step.contains === "string" ? step.contains : undefined,
				equals: typeof step.equals === "string" ? step.equals : undefined,
				raw: Boolean(step.raw),
				selector: step.selector,
				timeout: typeof step.timeout === "number" ? step.timeout : undefined,
			});
		case "element.assert-attr":
			if (typeof step.selector !== "string" || typeof step.name !== "string") {
				throw new CliError(
					"Batch step element.assert-attr requires `selector` and `name`.",
					{ command: "batch" },
				);
			}
			return elementAssertAttr(target, {
				contains: typeof step.contains === "string" ? step.contains : undefined,
				equals: typeof step.equals === "string" ? step.equals : undefined,
				exists: Boolean(step.exists),
				name: step.name,
				selector: step.selector,
				timeout: typeof step.timeout === "number" ? step.timeout : undefined,
			});
		case "app.wait-ready":
			return appWaitReady(target, {
				timeout: typeof step.timeout === "number" ? step.timeout : undefined,
			});
		case "app.status":
			return appStatus(target);
		case "app.service-status":
			return appServiceStatus(target, {
				service: typeof step.service === "string" ? step.service : undefined,
			});
		default:
			throw new CliError(`Unsupported batch step command: ${command}`, {
				command: "batch",
			});
	}
}

function readBatchSteps(file?: string) {
	const contents = file
		? readFileSync(path.resolve(file), "utf8")
		: !process.stdin.isTTY
			? readFileSync(0, "utf8")
			: null;
	if (contents === null) {
		throw new CliError(
			"Provide batch steps with --file <path> or JSON on stdin.",
			{
				command: "batch",
			},
		);
	}

	const parsed = JSON.parse(contents) as BatchStep[] | { steps: BatchStep[] };
	const steps = Array.isArray(parsed) ? parsed : parsed.steps;
	if (!Array.isArray(steps) || steps.length === 0) {
		throw new CliError(
			"Batch payload must be a non-empty JSON array or an object with a `steps` array.",
			{ command: "batch" },
		);
	}

	for (const step of steps) {
		if (!step || typeof step !== "object" || typeof step.command !== "string") {
			throw new CliError(
				"Each batch step must be an object with a string `command` field.",
				{ command: "batch" },
			);
		}
	}

	return steps;
}

async function runCommand(
	commandName: string,
	flags: Record<string, unknown>,
	handler: () => Promise<CommandResult>,
) {
	const startedAt = new Date().toISOString();
	const startedAtMs = Date.now();
	let session = tryResolveLoggingSession(commandName, flags);

	try {
		const result = await handler();
		printJson(result);
		session = sessionFromResult(result) ?? session;
		logCommandAttempt(commandName, flags, startedAt, startedAtMs, {
			error: null,
			result,
			session,
		});
	} catch (error) {
		const errorSession =
			error instanceof DriverLaunchError ? error.session : session;
		printError(error);
		logCommandAttempt(commandName, flags, startedAt, startedAtMs, {
			error: error instanceof Error ? error.message : String(error),
			session: errorSession,
		});
		process.exitCode = 1;
	}
}

function addExamples(command: Command, examples: string[]) {
	command.addHelpText(
		"after",
		`\nExamples:\n${examples.map((example) => `  ${example}`).join("\n")}`,
	);
}

function lastCommand(commands: readonly Command[]) {
	const command = commands[commands.length - 1];
	if (!command) {
		throw new Error(
			"Expected a command to be registered before adding examples.",
		);
	}
	return command;
}

function buildProgram() {
	const program = new Command();
	program
		.name("driver")
		.description("Silo driver CLI")
		.configureOutput({
			writeErr: (value) => process.stderr.write(value),
			writeOut: (value) => process.stderr.write(value),
		})
		.exitOverride()
		.showHelpAfterError()
		.showSuggestionAfterError()
		.addHelpText(
			"after",
			[
				"",
				"Workflow:",
				"  bun run driver -- session launch",
				"  bun run driver -- app status --session <id>",
				"  bun run driver -- page snapshot --session <id>",
				"  bun run driver -- session close --session <id>",
			].join("\n"),
		);

	program
		.command("help [command...]")
		.summary("Show general or command-specific help.")
		.action(async (commandPath: string[] | undefined) => {
			const tokens = commandPath ?? [];
			await runCommand("help", { commandPath: tokens }, async () => {
				const target = tokens.length
					? resolveCommandByPath(program, tokens)
					: program;
				process.stderr.write(`${target.helpInformation()}\n`);
				return tokens.length
					? commandJson(target)
					: {
							commands: program.commands.map((command) => ({
								command: dottedCommandPath(command),
								summary: commandSummary(command),
							})),
						};
			});
		});

	program
		.command("schema [command...]")
		.summary("Return machine-readable command metadata.")
		.action(async (commandPath: string[] | undefined) => {
			const tokens = commandPath ?? [];
			await runCommand("schema", { commandPath: tokens }, async () => {
				if (tokens.length > 0) {
					return commandJson(resolveCommandByPath(program, tokens));
				}
				return {
					commands: program.commands.map((command) => commandJson(command)),
				};
			});
		});

	program
		.command("history")
		.summary("Read recent driver command attempts from the command log.")
		.addOption(sessionOption())
		.addOption(sourceStateDirOption())
		.addOption(
			new Option("--command <name>", "Only include attempts for one command."),
		)
		.addOption(
			new Option("--limit <count>", "Maximum number of entries to return.")
				.argParser(parseIntegerOption("limit"))
				.default(20),
		)
		.action(
			async (options: {
				command?: string;
				limit: number;
				session?: string;
				sourceStateDir?: string;
			}) => {
				const flags = normalizeFlags(options);
				await runCommand("history", flags, async () => {
					const session = resolveOptionalSession("history", options);
					const sourceStateDir =
						options.sourceStateDir ??
						session?.sourceStateDir ??
						defaultSourceStateDir;
					const commandLogPath =
						session?.driverLogPath ??
						globalDriverCommandLogPath(sourceStateDir);
					const entries = (
						existsSync(commandLogPath)
							? readFileSync(commandLogPath, "utf8")
									.split("\n")
									.map((line) => line.trim())
									.filter((line) => line.length > 0)
									.map((line) => JSON.parse(line) as DriverCommandLogEntry)
							: []
					)
						.filter((entry) =>
							options.command ? entry.command === options.command : true,
						)
						.slice(-options.limit)
						.reverse();
					return { commandLogPath, entries };
				});
			},
		);
	addExamples(lastCommand(program.commands), [
		"bun run driver -- history",
		"bun run driver -- history --limit 50 --command element.click",
	]);

	program
		.command("batch")
		.summary("Run multiple page/app operations over one CDP attachment.")
		.addOption(sessionOption().makeOptionMandatory())
		.addOption(
			new Option("--file <path>", "Read batch steps from a JSON file."),
		)
		.action(async (options: { file?: string; session?: string }) => {
			const flags = normalizeFlags(options);
			await runCommand("batch", flags, async () =>
				withConnectedSession("batch", options, async (target) => {
					const steps = readBatchSteps(options.file);
					const results = [];
					for (const [index, step] of steps.entries()) {
						try {
							results.push({
								command: normalizeBatchCommandName(step.command),
								index,
								result: await runBatchStep(target, step),
							});
						} catch (error) {
							throw new CliError(
								`Batch step ${index} (${step.command}) failed: ${error instanceof Error ? error.message : String(error)}`,
								{ command: "batch" },
							);
						}
					}
					return {
						results,
						sessionId: target.session.id,
						stepCount: steps.length,
					};
				}),
			);
		});
	addExamples(lastCommand(program.commands), [
		"cat steps.json | bun run driver -- batch --session <id>",
		"bun run driver -- batch --session <id> --file ./steps.json",
	]);

	const session = program.command("session").summary("Manage driver sessions.");
	session
		.command("launch")
		.summary("Launch Silo, wait for readiness, and persist a reusable session.")
		.addOption(new Option("--id <id>", "Stable session id / trace id."))
		.addOption(
			new Option("--cdp-port <port>", "Explicit CDP port.").argParser(
				parseIntegerOption("cdp-port"),
			),
		)
		.addOption(timeoutOption())
		.addOption(sourceStateDirOption())
		.addOption(
			new Option(
				"--skip-preflight",
				"Skip the live preflight checks before launch.",
			),
		)
		.action(
			async (options: {
				cdpPort?: number;
				id?: string;
				skipPreflight?: boolean;
				sourceStateDir?: string;
				timeout?: number;
			}) => {
				const flags = normalizeFlags(options);
				await runCommand("session.launch", flags, async () => {
					let launched: Awaited<ReturnType<typeof launchDriverSession>> | null =
						null;
					try {
						launched = await launchDriverSession({
							cdpPort: options.cdpPort,
							id: options.id,
							skipPreflight: options.skipPreflight ?? false,
							sourceStateDir: options.sourceStateDir,
						});
						const status = await waitForAppReady(
							launched.page,
							options.timeout ?? 60_000,
						);
						const sessionFile = writeSessionRecord(launched.session);
						return {
							session: launched.session,
							sessionEnv: {
								SILO_DRIVER_SESSION: launched.session.id,
							},
							sessionFile,
							sessionId: launched.session.id,
							status,
						};
					} catch (error) {
						if (launched) {
							await stopLaunchedSession(launched.session);
							throw new DriverLaunchError(
								error instanceof Error ? error.message : String(error),
								launched.session,
							);
						}
						throw error;
					} finally {
						if (launched) {
							await disconnectFromDriverSession(launched);
						}
					}
				});
			},
		);
	addExamples(lastCommand(session.commands), [
		"bun run driver -- session launch",
		"bun run driver -- session launch --id local-smoke --cdp-port 9333",
	]);

	session
		.command("close")
		.summary("Stop a launched app session and remove its session record.")
		.addOption(sessionOption().makeOptionMandatory())
		.action(async (options: SessionOptions) => {
			const flags = normalizeFlags(options);
			await runCommand("session.close", flags, async () => {
				const resolved = resolveRequiredSession("session.close", options);
				await stopLaunchedSession(resolved);
				removeSessionRecord(resolved.id);
				return { closed: true, sessionId: resolved.id };
			});
		});
	addExamples(lastCommand(session.commands), [
		"bun run driver -- session close --session <id>",
		"bun run driver -- session close --session latest",
	]);

	session
		.command("status")
		.summary("Report session connectivity and process health.")
		.addOption(sessionOption().makeOptionMandatory())
		.action(async (options: SessionOptions) => {
			const flags = normalizeFlags(options);
			await runCommand("session.status", flags, async () => {
				const resolved = resolveRequiredSession("session.status", options);
				try {
					const connected = await connectToDriverSession(resolved);
					try {
						return {
							connected: true,
							pageTitle: await connected.page.title().catch(() => ""),
							pageUrl: connected.page.url(),
							session: resolved,
							tauriRunning: isPidRunning(resolved.tauriPid),
							viteRunning: resolved.vitePid
								? isPidRunning(resolved.vitePid)
								: null,
						};
					} finally {
						await disconnectFromDriverSession(connected);
					}
				} catch (error) {
					return {
						connected: false,
						error: error instanceof Error ? error.message : String(error),
						session: resolved,
						tauriRunning: isPidRunning(resolved.tauriPid),
						viteRunning: resolved.vitePid
							? isPidRunning(resolved.vitePid)
							: null,
					};
				}
			});
		});
	addExamples(lastCommand(session.commands), [
		"bun run driver -- session status --session <id>",
		"bun run driver -- session status --session latest",
	]);

	session
		.command("list")
		.summary(
			"List known driver sessions and whether their processes are still alive.",
		)
		.action(async () => {
			await runCommand("session.list", {}, async () => ({
				sessions: listSessionRecords().map((entry) => ({
					...entry,
					tauriRunning: isPidRunning(entry.tauriPid),
					viteRunning: entry.vitePid ? isPidRunning(entry.vitePid) : null,
				})),
			}));
		});
	addExamples(lastCommand(session.commands), [
		"bun run driver -- session list",
	]);

	const page = program
		.command("page")
		.summary("Inspect or drive the active page.");
	page
		.command("snapshot")
		.summary("Return an accessibility snapshot of the active page.")
		.addOption(sessionOption().makeOptionMandatory())
		.action(async (options: SessionOptions) => {
			const flags = normalizeFlags(options);
			await runCommand("page.snapshot", flags, async () =>
				withConnectedSession("page.snapshot", options, pageSnapshot),
			);
		});
	addExamples(lastCommand(page.commands), [
		"bun run driver -- page snapshot --session <id>",
	]);

	page
		.command("screenshot")
		.summary("Capture a screenshot of the active page.")
		.addOption(sessionOption().makeOptionMandatory())
		.addOption(new Option("--output <path>", "Path for the screenshot file."))
		.addOption(new Option("--full-page", "Capture the full scrollable page."))
		.action(
			async (
				options: SessionOptions & { fullPage?: boolean; output?: string },
			) => {
				const flags = normalizeFlags(options);
				await runCommand("page.screenshot", flags, async () =>
					withConnectedSession("page.screenshot", options, async (target) =>
						pageScreenshot(target, options),
					),
				);
			},
		);
	addExamples(lastCommand(page.commands), [
		"bun run driver -- page screenshot --session <id>",
		"bun run driver -- page screenshot --session <id> --output /tmp/silo.png --full-page",
	]);

	page
		.command("console")
		.summary("Read webview console entries from the current session log.")
		.addOption(sessionOption().makeOptionMandatory())
		.addOption(new Option("--level <level>", "Minimum console level."))
		.action(async (options: SessionOptions & { level?: string }) => {
			const flags = normalizeFlags(options);
			await runCommand("page.console", flags, async () => {
				const resolved = resolveRequiredSession("page.console", options);
				return {
					entries: readConsoleEntries(resolved, options.level),
					sessionId: resolved.id,
				};
			});
		});
	addExamples(lastCommand(page.commands), [
		"bun run driver -- page console --session <id>",
		"bun run driver -- page console --session <id> --level warn",
	]);

	page
		.command("network")
		.summary("Return network resource timings from the active page.")
		.addOption(sessionOption().makeOptionMandatory())
		.action(async (options: SessionOptions) => {
			const flags = normalizeFlags(options);
			await runCommand("page.network", flags, async () =>
				withConnectedSession("page.network", options, pageNetwork),
			);
		});
	addExamples(lastCommand(page.commands), [
		"bun run driver -- page network --session <id>",
	]);

	page
		.command("press")
		.summary("Press a keyboard key or shortcut on the active page.")
		.requiredOption("--key <key>", "Keyboard key or shortcut.")
		.addOption(sessionOption().makeOptionMandatory())
		.action(async (options: SessionOptions & { key: string }) => {
			const flags = normalizeFlags(options);
			await runCommand("page.press", flags, async () =>
				withConnectedSession("page.press", options, async (target) =>
					pagePress(target, { key: options.key }),
				),
			);
		});
	addExamples(lastCommand(page.commands), [
		"bun run driver -- page press --session <id> --key Enter",
		"bun run driver -- page press --session <id> --key Meta+K",
	]);

	page
		.command("eval")
		.summary("Evaluate a JavaScript expression in the active page.")
		.requiredOption("--js <expression>", "JavaScript expression to evaluate.")
		.addOption(sessionOption().makeOptionMandatory())
		.action(async (options: SessionOptions & { js: string }) => {
			const flags = normalizeFlags(options);
			await runCommand("page.eval", flags, async () =>
				withConnectedSession("page.eval", options, async (target) =>
					pageEval(target, { js: options.js }),
				),
			);
		});
	addExamples(lastCommand(page.commands), [
		"bun run driver -- page eval --session <id> --js 'document.title'",
	]);

	const element = program
		.command("element")
		.summary("Read or interact with a matched locator.");

	element
		.command("click")
		.summary("Click a matching element.")
		.requiredOption("--selector <selector>", "Target selector.")
		.addOption(sessionOption().makeOptionMandatory())
		.addOption(timeoutOption())
		.action(
			async (
				options: SessionOptions & TimeoutOptions & { selector: string },
			) => {
				const flags = normalizeFlags(options);
				await runCommand("element.click", flags, async () =>
					withConnectedSession("element.click", options, async (target) =>
						elementClick(target, options),
					),
				);
			},
		);
	addExamples(lastCommand(element.commands), [
		"bun run driver -- element click --session <id> --selector testid:dashboard-action-open-project",
		"bun run driver -- element click --session <id> --selector 'role:button[name=\"Open Project\"]'",
	]);

	element
		.command("type")
		.summary("Fill or type text into a matching element.")
		.requiredOption("--selector <selector>", "Target selector.")
		.requiredOption("--text <text>", "Text to enter.")
		.addOption(sessionOption().makeOptionMandatory())
		.addOption(timeoutOption())
		.addOption(new Option("--slowly", "Type one character at a time."))
		.addOption(new Option("--submit", "Press Enter after typing."))
		.action(
			async (
				options: SessionOptions &
					TimeoutOptions & {
						selector: string;
						slowly?: boolean;
						submit?: boolean;
						text: string;
					},
			) => {
				const flags = normalizeFlags(options);
				await runCommand("element.type", flags, async () =>
					withConnectedSession("element.type", options, async (target) =>
						elementType(target, options),
					),
				);
			},
		);
	addExamples(lastCommand(element.commands), [
		"bun run driver -- element type --session <id> --selector 'label:Project path' --text /tmp/repo",
		"bun run driver -- element type --session <id> --selector css:textarea --text prompt --slowly --submit",
	]);

	element
		.command("wait")
		.summary("Wait for a selector to reach a given state.")
		.requiredOption("--selector <selector>", "Target selector.")
		.addOption(sessionOption().makeOptionMandatory())
		.addOption(timeoutOption())
		.addOption(
			new Option(
				"--state <state>",
				"One of: visible, hidden, attached, detached.",
			).choices(["visible", "hidden", "attached", "detached"]),
		)
		.action(
			async (
				options: SessionOptions &
					TimeoutOptions & {
						selector: string;
						state?: "attached" | "detached" | "hidden" | "visible";
					},
			) => {
				const flags = normalizeFlags(options);
				await runCommand("element.wait", flags, async () =>
					withConnectedSession("element.wait", options, async (target) =>
						elementWait(target, options),
					),
				);
			},
		);
	addExamples(lastCommand(element.commands), [
		"bun run driver -- element wait --session <id> --selector testid:setup-status-gcloud",
		"bun run driver -- element wait --session <id> --selector text:Saved --state visible",
	]);

	element
		.command("text")
		.summary("Read the text content of the first matching element.")
		.requiredOption("--selector <selector>", "Target selector.")
		.addOption(sessionOption().makeOptionMandatory())
		.addOption(timeoutOption())
		.addOption(new Option("--raw", "Do not trim whitespace."))
		.action(
			async (
				options: SessionOptions &
					TimeoutOptions & { raw?: boolean; selector: string },
			) => {
				const flags = normalizeFlags(options);
				await runCommand("element.text", flags, async () =>
					withConnectedSession("element.text", options, async (target) =>
						elementText(target, options),
					),
				);
			},
		);
	addExamples(lastCommand(element.commands), [
		"bun run driver -- element text --session <id> --selector testid:setup-status-gcloud",
		"bun run driver -- element text --session <id> --selector css:main h1",
	]);

	element
		.command("html")
		.summary("Read the outer HTML of the first matching element.")
		.requiredOption("--selector <selector>", "Target selector.")
		.addOption(sessionOption().makeOptionMandatory())
		.addOption(timeoutOption())
		.action(
			async (
				options: SessionOptions & TimeoutOptions & { selector: string },
			) => {
				const flags = normalizeFlags(options);
				await runCommand("element.html", flags, async () =>
					withConnectedSession("element.html", options, async (target) =>
						elementHtml(target, options),
					),
				);
			},
		);
	addExamples(lastCommand(element.commands), [
		"bun run driver -- element html --session <id> --selector css:main",
	]);

	element
		.command("attr")
		.summary("Read an attribute from the first matching element.")
		.requiredOption("--selector <selector>", "Target selector.")
		.requiredOption("--name <attribute>", "Attribute name.")
		.addOption(sessionOption().makeOptionMandatory())
		.addOption(timeoutOption())
		.action(
			async (
				options: SessionOptions &
					TimeoutOptions & { name: string; selector: string },
			) => {
				const flags = normalizeFlags(options);
				await runCommand("element.attr", flags, async () =>
					withConnectedSession("element.attr", options, async (target) =>
						elementAttr(target, options),
					),
				);
			},
		);
	addExamples(lastCommand(element.commands), [
		"bun run driver -- element attr --session <id> --selector testid:setup-status-gcloud --name data-status-label",
	]);

	element
		.command("exists")
		.summary(
			"Check whether a selector exists, optionally requiring visibility.",
		)
		.requiredOption("--selector <selector>", "Target selector.")
		.addOption(sessionOption().makeOptionMandatory())
		.addOption(
			new Option("--visible", "Require the first match to be visible."),
		)
		.action(
			async (
				options: SessionOptions & { selector: string; visible?: boolean },
			) => {
				const flags = normalizeFlags(options);
				await runCommand("element.exists", flags, async () =>
					withConnectedSession("element.exists", options, async (target) =>
						elementExists(target, options),
					),
				);
			},
		);
	addExamples(lastCommand(element.commands), [
		"bun run driver -- element exists --session <id> --selector testid:setup-status-github",
		"bun run driver -- element exists --session <id> --selector css:dialog --visible",
	]);

	element
		.command("count")
		.summary("Count matching elements.")
		.requiredOption("--selector <selector>", "Target selector.")
		.addOption(sessionOption().makeOptionMandatory())
		.action(async (options: SessionOptions & { selector: string }) => {
			const flags = normalizeFlags(options);
			await runCommand("element.count", flags, async () =>
				withConnectedSession("element.count", options, async (target) =>
					elementCount(target, options),
				),
			);
		});
	addExamples(lastCommand(element.commands), [
		"bun run driver -- element count --session <id> --selector role:button",
	]);

	element
		.command("assert")
		.summary(
			"Assert a selector state such as visible, hidden, enabled, or disabled.",
		)
		.requiredOption("--selector <selector>", "Target selector.")
		.addOption(sessionOption().makeOptionMandatory())
		.addOption(timeoutOption())
		.addOption(new Option("--visible", "Assert the locator is visible."))
		.addOption(new Option("--hidden", "Assert the locator is hidden."))
		.addOption(new Option("--attached", "Assert the locator is attached."))
		.addOption(new Option("--detached", "Assert the locator is detached."))
		.addOption(new Option("--enabled", "Assert the locator is enabled."))
		.addOption(new Option("--disabled", "Assert the locator is disabled."))
		.action(
			async (
				options: SessionOptions &
					TimeoutOptions & {
						attached?: boolean;
						detached?: boolean;
						disabled?: boolean;
						enabled?: boolean;
						hidden?: boolean;
						selector: string;
						visible?: boolean;
					},
			) => {
				const flags = normalizeFlags(options);
				await runCommand("element.assert", flags, async () =>
					withConnectedSession("element.assert", options, async (target) =>
						elementAssert(target, options),
					),
				);
			},
		);
	addExamples(lastCommand(element.commands), [
		"bun run driver -- element assert --session <id> --selector text:Open Project --visible",
		"bun run driver -- element assert --session <id> --selector css:.toast --hidden",
	]);

	element
		.command("assert-text")
		.summary(
			"Assert text equality or containment on the first matching element.",
		)
		.requiredOption("--selector <selector>", "Target selector.")
		.addOption(sessionOption().makeOptionMandatory())
		.addOption(timeoutOption())
		.addOption(new Option("--equals <text>", "Require exact text equality."))
		.addOption(
			new Option("--contains <text>", "Require substring containment."),
		)
		.addOption(new Option("--raw", "Do not trim whitespace before comparing."))
		.action(
			async (
				options: SessionOptions &
					TimeoutOptions & {
						contains?: string;
						equals?: string;
						raw?: boolean;
						selector: string;
					},
			) => {
				const flags = normalizeFlags(options);
				await runCommand("element.assert-text", flags, async () =>
					withConnectedSession("element.assert-text", options, async (target) =>
						elementAssertText(target, options),
					),
				);
			},
		);
	addExamples(lastCommand(element.commands), [
		"bun run driver -- element assert-text --session <id> --selector css:h1 --equals Dashboard",
		"bun run driver -- element assert-text --session <id> --selector testid:setup-status-gcloud --contains connected",
	]);

	element
		.command("assert-attr")
		.summary("Assert an attribute exists, equals a value, or contains a value.")
		.requiredOption("--selector <selector>", "Target selector.")
		.requiredOption("--name <attribute>", "Attribute name.")
		.addOption(sessionOption().makeOptionMandatory())
		.addOption(timeoutOption())
		.addOption(new Option("--exists", "Require the attribute to exist."))
		.addOption(new Option("--equals <value>", "Require exact equality."))
		.addOption(
			new Option("--contains <value>", "Require substring containment."),
		)
		.action(
			async (
				options: SessionOptions &
					TimeoutOptions & {
						contains?: string;
						equals?: string;
						exists?: boolean;
						name: string;
						selector: string;
					},
			) => {
				const flags = normalizeFlags(options);
				await runCommand("element.assert-attr", flags, async () =>
					withConnectedSession("element.assert-attr", options, async (target) =>
						elementAssertAttr(target, options),
					),
				);
			},
		);
	addExamples(lastCommand(element.commands), [
		"bun run driver -- element assert-attr --session <id> --selector css:button --name disabled --exists",
		"bun run driver -- element assert-attr --session <id> --selector testid:setup-status-gcloud --name data-status-label --contains connected",
	]);

	const app = program.command("app").summary("Read app-specific status.");
	app
		.command("wait-ready")
		.summary(
			"Wait for the Silo dashboard and service statuses to become ready.",
		)
		.addOption(sessionOption().makeOptionMandatory())
		.addOption(timeoutOption())
		.action(async (options: SessionOptions & TimeoutOptions) => {
			const flags = normalizeFlags(options);
			await runCommand("app.wait-ready", flags, async () =>
				withConnectedSession("app.wait-ready", options, async (target) =>
					appWaitReady(target, options),
				),
			);
		});
	addExamples(lastCommand(app.commands), [
		"bun run driver -- app wait-ready --session <id>",
	]);

	app
		.command("status")
		.summary("Return high-level app status for the active page.")
		.addOption(sessionOption().makeOptionMandatory())
		.action(async (options: SessionOptions) => {
			const flags = normalizeFlags(options);
			await runCommand("app.status", flags, async () =>
				withConnectedSession("app.status", options, appStatus),
			);
		});
	addExamples(lastCommand(app.commands), [
		"bun run driver -- app status --session <id>",
	]);

	app
		.command("service-status")
		.summary("Return dashboard service connectivity labels.")
		.addOption(sessionOption().makeOptionMandatory())
		.addOption(
			new Option(
				"--service <service>",
				"One of gcloud, github, codex, claude.",
			),
		)
		.action(async (options: SessionOptions & { service?: string }) => {
			const flags = normalizeFlags(options);
			await runCommand("app.service-status", flags, async () =>
				withConnectedSession("app.service-status", options, async (target) =>
					appServiceStatus(target, options),
				),
			);
		});
	addExamples(lastCommand(app.commands), [
		"bun run driver -- app service-status --session <id>",
		"bun run driver -- app service-status --session <id> --service gcloud",
	]);

	const video = program
		.command("video")
		.summary("Inspect trace video artifacts.");
	video
		.command("status")
		.summary("Report the current session's trace video paths and file status.")
		.addOption(sessionOption())
		.addOption(sourceStateDirOption())
		.addOption(
			new Option("--trace-id <traceId>", "Inspect a trace by id after close."),
		)
		.action(
			async (
				options: SessionOptions & {
					sourceStateDir?: string;
					traceId?: string;
				},
			) => {
				const flags = normalizeFlags(options);
				await runCommand("video.status", flags, async () => {
					const resolvedSession = resolveOptionalSession(
						"video.status",
						options,
					);
					const resolvedTraceId = options.traceId ?? resolvedSession?.traceId;
					if (!resolvedTraceId) {
						throw new CliError("Provide either --session or --trace-id.", {
							command: "video.status",
						});
					}

					const sourceStateDir =
						options.sourceStateDir ?? defaultSourceStateDir;
					const traceDir =
						resolvedSession?.traceDir ??
						traceDirFor(sourceStateDir, resolvedTraceId);
					const videoMetadataPath =
						resolvedSession?.videoMetadataPath ??
						path.join(traceDir, "video-metadata.json");
					const videoPath =
						resolvedSession?.videoPath ?? path.join(traceDir, "video.mp4");
					const metadataExists = existsSync(videoMetadataPath);
					const videoExists = existsSync(videoPath);
					const metadata = metadataExists
						? JSON.parse(readFileSync(videoMetadataPath, "utf8"))
						: null;
					return {
						metadata,
						sessionId: resolvedSession?.id ?? null,
						traceId: resolvedTraceId,
						videoExists,
						videoMetadataExists: metadataExists,
						videoMetadataPath,
						videoPath,
						videoRecorderPid: resolvedSession?.videoRecorderPid ?? null,
					};
				});
			},
		);
	addExamples(lastCommand(video.commands), [
		"bun run driver -- video status --session <id>",
		"bun run driver -- video status --trace-id trace-smoke",
	]);

	return program;
}

async function main() {
	currentUserArgv = process.argv.slice(2);
	const program = buildProgram();

	try {
		await program.parseAsync(currentUserArgv, { from: "user" });
	} catch (error) {
		if (error instanceof CommanderError) {
			if (error.code === "commander.helpDisplayed") {
				const commandPath = currentUserArgv.filter(
					(token) => !token.startsWith("--"),
				);
				const helpPath =
					commandPath[0] === "help"
						? commandPath.slice(1)
						: commandPath.length > 0
							? commandPath
							: [];
				try {
					const target = helpPath.length
						? resolveCommandByPath(program, helpPath)
						: program;
					printJson(
						helpPath.length
							? commandJson(target)
							: {
									commands: program.commands.map((command) => ({
										command: dottedCommandPath(command),
										summary: commandSummary(command),
									})),
								},
					);
				} catch (helpError) {
					printError(helpError);
				}
				return;
			}

			printCommanderError(error);
			process.exitCode = 1;
			return;
		}

		printError(error);
		process.exitCode = 1;
	}
}

void main();
