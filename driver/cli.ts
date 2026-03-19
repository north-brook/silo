import { appendFileSync, existsSync, readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import type { BrowserContext, Locator, Page } from "@playwright/test";
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

type ParsedArgs = {
	command: string;
	flags: Map<string, string | boolean>;
};

type CommandResult = Record<string, unknown>;

type CommandHandler = (args: ParsedArgs) => Promise<CommandResult>;

type CommandDefinition = {
	summary: string;
	usage: string[];
	examples: string[];
	handler: CommandHandler;
};

type DriverCommandLogEntry = {
	argv: string[];
	command: string;
	durationMs: number;
	error: string | null;
	flags: Record<string, string | boolean>;
	ok: boolean;
	pid: number;
	startedAt: string;
};

type ConnectedCommandTarget = {
	context: BrowserContext;
	page: Page;
	session: DriverSessionRecord;
	tabIndex: number;
};

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

function parseCommand(argv: string[]): ParsedArgs {
	const [first = "help", second, ...rest] = argv;
	let command = first;
	let tokens = second === undefined ? [] : [second, ...rest];

	if (first === "help") {
		command = "help";
		tokens = second && !second.startsWith("--") ? ["--command", second, ...rest] : rest;
	} else if (
		(first === "app" || first === "tabs" || first === "sessions") &&
		second &&
		!second.startsWith("--")
	) {
		command = `${first}.${second}`;
		tokens = rest;
	}

	const flags = new Map<string, string | boolean>();
	for (let index = 0; index < tokens.length; index += 1) {
		const token = tokens[index];
		if (token === "-h") {
			flags.set("help", true);
			continue;
		}
		if (!token.startsWith("--")) {
			continue;
		}

		const key = token.slice(2);
		const next = tokens[index + 1];
		if (!next || next.startsWith("--")) {
			flags.set(key, true);
			continue;
		}

		flags.set(key, next);
		index += 1;
	}

	if (flags.get("help") === true && command !== "help") {
		return {
			command: "help",
			flags: new Map([
				["command", command],
				...Array.from(flags.entries()).filter(([key]) => key !== "help"),
			]),
		};
	}

	return { command, flags };
}

function flag(args: ParsedArgs, name: string) {
	return args.flags.get(name);
}

function requireStringFlag(
	args: ParsedArgs,
	name: string,
	options: { command?: string } = {},
) {
	const value = flag(args, name);
	if (typeof value !== "string" || value.length === 0) {
		throw new CliError(`Missing required flag --${name}`, {
			command: options.command ?? args.command,
			hint: `Run \`bun run driver -- help ${options.command ?? args.command}\` for usage.`,
		});
	}

	return value;
}

function optionalNumberFlag(args: ParsedArgs, name: string) {
	const value = flag(args, name);
	if (typeof value !== "string" || value.length === 0) {
		return undefined;
	}

	const parsed = Number.parseInt(value, 10);
	if (!Number.isFinite(parsed)) {
		throw new CliError(`Invalid numeric value for --${name}: ${value}`, {
			command: args.command,
			hint: `Run \`bun run driver -- help ${args.command}\` for usage.`,
		});
	}

	return parsed;
}

function booleanFlag(args: ParsedArgs, name: string) {
	return flag(args, name) === true;
}

function latestSessionLog(session: DriverSessionRecord) {
	if (typeof session.appLogPath === "string" && existsSync(session.appLogPath)) {
		return session.appLogPath;
	}

	const logsDir = path.join(session.stateDir, "logs");
	const latest = readdirSync(logsDir)
		.filter((entry) => entry.endsWith(".log"))
		.sort()
		.pop();

	if (!latest) {
		throw new CliError(`No session log files found in ${logsDir}`, {
			command: "console",
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

function assertKnownCommand(command: string) {
	if (command in commandDefinitions) {
		return;
	}

	throw new CliError(`Unknown command: ${command}`, {
		hint: "Run `bun run driver -- help` to list available commands.",
	});
}

function commandHelpLines(commandName: string) {
	const definition = commandDefinitions[commandName];
	if (!definition) {
		throw new CliError(`Unknown command: ${commandName}`, {
			hint: "Run `bun run driver -- help` to list available commands.",
		});
	}

	return [
		`${commandName}: ${definition.summary}`,
		"",
		"Usage:",
		...definition.usage.map((usage) => `  ${usage}`),
		"",
		"Examples:",
		...definition.examples.map((example) => `  ${example}`),
	].join("\n");
}

function generalHelpLines() {
	return [
		"Silo driver CLI",
		"",
		"Core workflow:",
		"  bun run driver -- launch",
		"  bun run driver -- app.wait-ready",
		"  bun run driver -- app.status",
		"  bun run driver -- close",
		"",
		"Commands:",
		...Object.entries(commandDefinitions)
			.filter(([name]) => name !== "help")
			.map(([name, definition]) => `  ${name.padEnd(16)} ${definition.summary}`),
		"",
		"Get command-specific help:",
		"  bun run driver -- help <command>",
	].join("\n");
}

function printHelpText(commandName?: string) {
	process.stderr.write(`${commandName ? commandHelpLines(commandName) : generalHelpLines()}\n`);
}

function locatorSummary(selector: string) {
	return {
		parsedSelector: parseSelector(selector),
		selector,
	};
}

function attachFailure(command: string, session: DriverSessionRecord, error: unknown) {
	const message = error instanceof Error ? error.message : String(error);
	return new CliError(
		`Failed to attach to driver session ${session.id} over CDP. ${message}`,
		{
			command,
			hint: `The session may be stale or the app may have exited. Run \`bun run driver -- status --session ${session.id}\` or launch a fresh session with \`bun run driver -- launch\`.`,
		},
	);
}

async function resolveTargetPage(
	context: BrowserContext,
	defaultPage: Page,
	requestedIndex?: number,
) {
	const pages = context.pages();
	const defaultIndex = pages.findIndex((candidate) => candidate === defaultPage);
	const tabIndex = requestedIndex ?? Math.max(defaultIndex, 0);

	if (tabIndex < 0 || tabIndex >= pages.length) {
		throw new CliError(`Tab index ${tabIndex} is out of range.`, {
			hint: `Run \`bun run driver -- tabs.list\` first. Available tabs: ${pages.length}.`,
		});
	}

	const page = pages[tabIndex];
	await page.waitForLoadState("domcontentloaded").catch(() => undefined);
	return { page, tabIndex };
}

async function withConnectedSession<T extends CommandResult>(
	args: ParsedArgs,
	callback: (target: ConnectedCommandTarget) => Promise<T>,
) {
	const session = resolveSessionRecord(
		typeof flag(args, "session") === "string"
			? (flag(args, "session") as string)
			: undefined,
	);
	let connected: Awaited<ReturnType<typeof connectToDriverSession>>;
	try {
		connected = await connectToDriverSession(session);
	} catch (error) {
		throw attachFailure(args.command, session, error);
	}

	try {
		const { page, tabIndex } = await resolveTargetPage(
			connected.context,
			connected.page,
			optionalNumberFlag(args, "tab"),
		);
		return await callback({
			context: connected.context,
			page,
			session,
			tabIndex,
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

function globalDriverCommandLogPath(sourceStateDir: string) {
	return traceHistoryLogPath(sourceStateDir);
}

function readCommandLogEntries(commandLogPath: string) {
	if (!existsSync(commandLogPath)) {
		return [] as DriverCommandLogEntry[];
	}

	return readFileSync(commandLogPath, "utf8")
		.split("\n")
		.map((line) => line.trim())
		.filter((line) => line.length > 0)
		.map((line) => JSON.parse(line) as DriverCommandLogEntry);
}

function writeCommandLogEntry(commandLogPath: string, entry: DriverCommandLogEntry) {
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

function resolveLoggingSession(args: ParsedArgs) {
	if (
		args.command === "launch"
		|| args.command === "help"
		|| args.command === "history"
		|| args.command === "sessions.list"
	) {
		return undefined;
	}

	try {
		return resolveSessionRecord(
			typeof flag(args, "session") === "string"
				? (flag(args, "session") as string)
				: undefined,
		);
	} catch {
		return undefined;
	}
}

function logCommandAttempt(
	args: ParsedArgs,
	argv: string[],
	startedAt: string,
	startedAtMs: number,
	options: {
		error: string | null;
		result?: CommandResult;
		session?: DriverSessionRecord;
	},
) {
	const session = options.result ? sessionFromResult(options.result) ?? options.session : options.session;
	const sourceStateDir = session?.sourceStateDir
		?? (typeof flag(args, "source-state-dir") === "string"
			? (flag(args, "source-state-dir") as string)
			: defaultSourceStateDir);
	const entry: DriverCommandLogEntry = {
		argv,
		command: args.command,
		durationMs: Date.now() - startedAtMs,
		error: options.error,
		flags: Object.fromEntries(args.flags),
		ok: options.error === null,
		pid: process.pid,
		startedAt,
	};

	writeCommandLogEntry(globalDriverCommandLogPath(sourceStateDir), entry);
	if (session && typeof session.driverLogPath === "string") {
		writeCommandLogEntry(session.driverLogPath, entry);
	}
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

const commandDefinitions: Record<string, CommandDefinition> = {
	help: {
		summary: "Show general or command-specific help.",
		usage: [
			"bun run driver -- help",
			"bun run driver -- help <command>",
		],
		examples: [
			"bun run driver -- help click",
			"bun run driver -- help app.status",
		],
		async handler(args) {
			const command =
				typeof flag(args, "command") === "string"
					? (flag(args, "command") as string)
					: undefined;
			if (command) {
				assertKnownCommand(command);
				printHelpText(command);
				const definition = commandDefinitions[command];
				return {
					command,
					examples: definition.examples,
					summary: definition.summary,
					usage: definition.usage,
				};
			}

			printHelpText();
			return {
				commands: Object.entries(commandDefinitions)
					.filter(([name]) => name !== "help")
					.map(([name, definition]) => ({
						command: name,
						summary: definition.summary,
					})),
			};
		},
	},
	launch: {
		summary: "Launch Silo, attach over CDP, and persist a reusable session.",
		usage: [
			"bun run driver -- launch",
			"bun run driver -- launch --id local-smoke --cdp-port 9333",
		],
		examples: [
			"bun run driver -- launch",
			"bun run driver -- launch --source-state-dir ~/.silo",
		],
		async handler(args) {
			let launched: Awaited<ReturnType<typeof launchDriverSession>> | null = null;

			try {
				launched = await launchDriverSession({
					cdpPort: optionalNumberFlag(args, "cdp-port"),
					id:
						typeof flag(args, "id") === "string"
							? (flag(args, "id") as string)
							: undefined,
					skipPreflight: booleanFlag(args, "skip-preflight"),
					sourceStateDir:
						typeof flag(args, "source-state-dir") === "string"
							? (flag(args, "source-state-dir") as string)
							: undefined,
				});
				const status = await waitForAppReady(
					launched.page,
					optionalNumberFlag(args, "timeout") ?? 60_000,
				);
				const sessionFile = writeSessionRecord(launched.session);
				return {
					session: launched.session,
					sessionFile,
					status,
				};
			} catch (error) {
				if (launched) {
					const session = launched.session;
					await stopLaunchedSession(session);
					throw new DriverLaunchError(
						error instanceof Error ? error.message : String(error),
						session,
					);
				}
				throw error;
			} finally {
				if (launched) {
					await disconnectFromDriverSession(launched);
				}
			}
		},
	},
	close: {
		summary: "Stop the launched app session and remove its session record.",
		usage: [
			"bun run driver -- close",
			"bun run driver -- close --session <id>",
		],
		examples: [
			"bun run driver -- close",
			"bun run driver -- close --session 20260319-example",
		],
		async handler(args) {
			const session = resolveSessionRecord(
				typeof flag(args, "session") === "string"
					? (flag(args, "session") as string)
					: undefined,
			);
			await stopLaunchedSession(session);
			removeSessionRecord(session.id);
			return { closed: true, sessionId: session.id };
		},
	},
	status: {
		summary: "Report session connectivity and process health.",
		usage: [
			"bun run driver -- status",
			"bun run driver -- status --session <id>",
		],
		examples: [
			"bun run driver -- status",
			"bun run driver -- status --session 20260319-example",
		],
		async handler(args) {
			const session = resolveSessionRecord(
				typeof flag(args, "session") === "string"
					? (flag(args, "session") as string)
					: undefined,
			);
			try {
				const connected = await connectToDriverSession(session);
				try {
					return {
						connected: true,
						pageTitle: await connected.page.title().catch(() => ""),
						pageUrl: connected.page.url(),
						session,
						tauriRunning: isPidRunning(session.tauriPid),
						viteRunning: session.vitePid ? isPidRunning(session.vitePid) : null,
					};
				} finally {
					await disconnectFromDriverSession(connected);
				}
			} catch (error) {
				return {
					connected: false,
					error: error instanceof Error ? error.message : String(error),
					session,
					tauriRunning: isPidRunning(session.tauriPid),
					viteRunning: session.vitePid ? isPidRunning(session.vitePid) : null,
				};
			}
		},
	},
	"sessions.list": {
		summary: "List known driver sessions and whether their processes are still alive.",
		usage: ["bun run driver -- sessions.list"],
		examples: ["bun run driver -- sessions.list"],
		async handler() {
			return {
				sessions: listSessionRecords().map((session) => ({
					...session,
					tauriRunning: isPidRunning(session.tauriPid),
					viteRunning: session.vitePid ? isPidRunning(session.vitePid) : null,
				})),
			};
		},
	},
	history: {
		summary: "Read recent driver command attempts from the command log.",
		usage: [
			"bun run driver -- history",
			"bun run driver -- history --limit 20",
		],
		examples: [
			"bun run driver -- history",
			"bun run driver -- history --limit 50 --command help",
		],
		async handler(args) {
			const limit = optionalNumberFlag(args, "limit") ?? 20;
			const command =
				typeof flag(args, "command") === "string"
					? (flag(args, "command") as string)
					: undefined;
			const session =
				typeof flag(args, "session") === "string"
					? resolveSessionRecord(flag(args, "session") as string)
					: undefined;
			const sourceStateDir =
				typeof flag(args, "source-state-dir") === "string"
					? (flag(args, "source-state-dir") as string)
					: session?.sourceStateDir ?? defaultSourceStateDir;
			const commandLogPath =
				(typeof session?.driverLogPath === "string" ? session.driverLogPath : null)
				?? globalDriverCommandLogPath(sourceStateDir);
			const entries = readCommandLogEntries(commandLogPath)
				.filter((entry) => (command ? entry.command === command : true))
				.slice(-limit)
				.reverse();

			return {
				commandLogPath,
				entries,
			};
		},
	},
	snapshot: {
		summary: "Return an accessibility snapshot of the active page.",
		usage: [
			"bun run driver -- snapshot",
			"bun run driver -- snapshot --tab 1",
		],
		examples: [
			"bun run driver -- snapshot",
			"bun run driver -- snapshot --session <id> --tab 0",
		],
		async handler(args) {
			return withConnectedSession(args, async ({ page, session, tabIndex }) => ({
				pageUrl: page.url(),
				sessionId: session.id,
				snapshot: await page.locator("body").ariaSnapshot(),
				tabIndex,
			}));
		},
	},
	screenshot: {
		summary: "Capture a screenshot of the active page.",
		usage: [
			"bun run driver -- screenshot",
			"bun run driver -- screenshot --output ./shot.png --full-page",
		],
		examples: [
			"bun run driver -- screenshot",
			"bun run driver -- screenshot --tab 1 --output /tmp/silo.png",
		],
		async handler(args) {
			return withConnectedSession(args, async ({ page, session, tabIndex }) => {
				const output =
					typeof flag(args, "output") === "string"
						? path.resolve(flag(args, "output") as string)
						: path.join(session.artifactsDir, `screenshot-${Date.now()}.png`);
				await page.screenshot({
					fullPage: booleanFlag(args, "full-page"),
					path: output,
				});
				return { output, sessionId: session.id, tabIndex };
			});
		},
	},
	console: {
		summary: "Read webview console entries from the current session log.",
		usage: [
			"bun run driver -- console",
			"bun run driver -- console --level error",
		],
		examples: [
			"bun run driver -- console",
			"bun run driver -- console --session <id> --level warn",
		],
		async handler(args) {
			const session = resolveSessionRecord(
				typeof flag(args, "session") === "string"
					? (flag(args, "session") as string)
					: undefined,
			);
			const level =
				typeof flag(args, "level") === "string"
					? (flag(args, "level") as string)
					: undefined;
			return {
				entries: readConsoleEntries(session, level),
				sessionId: session.id,
			};
		},
	},
	network: {
		summary: "Return network resource timings from the active page.",
		usage: [
			"bun run driver -- network",
			"bun run driver -- network --tab 1",
		],
		examples: [
			"bun run driver -- network",
			"bun run driver -- network --session <id>",
		],
		async handler(args) {
			return withConnectedSession(args, async ({ page, session, tabIndex }) => ({
				pageUrl: page.url(),
				requests: await page.evaluate(() =>
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
				sessionId: session.id,
				tabIndex,
			}));
		},
	},
	click: {
		summary: "Click a matching element.",
		usage: [
			"bun run driver -- click --selector testid:dashboard-action-open-project",
			"bun run driver -- click --selector 'role:button[name=\"Open Project\"]'",
		],
		examples: [
			"bun run driver -- click --selector testid:dashboard-action-open-project",
			"bun run driver -- click --tab 1 --selector text:Continue",
		],
		async handler(args) {
			return withConnectedSession(args, async ({ page, session, tabIndex }) => {
				const selector = requireStringFlag(args, "selector");
				const locator = resolveLocator(page, selector);
				await locator.click({
					timeout: optionalNumberFlag(args, "timeout"),
				});
				return {
					action: "click",
					sessionId: session.id,
					tabIndex,
					...locatorSummary(selector),
				};
			});
		},
	},
	type: {
		summary: "Fill or type text into a matching element.",
		usage: [
			"bun run driver -- type --selector 'label:Project path' --text /tmp/repo",
			"bun run driver -- type --selector css:input --text hello --submit",
		],
		examples: [
			"bun run driver -- type --selector css:input --text hello",
			"bun run driver -- type --selector css:textarea --text prompt --slowly",
		],
		async handler(args) {
			return withConnectedSession(args, async ({ page, session, tabIndex }) => {
				const selector = requireStringFlag(args, "selector");
				const text = requireStringFlag(args, "text");
				const locator = resolveLocator(page, selector);
				const timeout = optionalNumberFlag(args, "timeout");
				await locator.waitFor({ state: "visible", timeout });
				if (booleanFlag(args, "slowly")) {
					await locator.click({ timeout });
					await locator.pressSequentially(text);
				} else {
					await locator.fill(text, { timeout });
				}
				if (booleanFlag(args, "submit")) {
					await page.keyboard.press("Enter");
				}
				return {
					action: "type",
					length: text.length,
					sessionId: session.id,
					tabIndex,
					...locatorSummary(selector),
				};
			});
		},
	},
	press: {
		summary: "Press a keyboard key or shortcut on the active page.",
		usage: [
			"bun run driver -- press --key Enter",
			"bun run driver -- press --key Meta+K",
		],
		examples: [
			"bun run driver -- press --key ArrowDown",
			"bun run driver -- press --key Enter",
		],
		async handler(args) {
			return withConnectedSession(args, async ({ page, session, tabIndex }) => {
				const key = requireStringFlag(args, "key");
				await page.keyboard.press(key);
				return { action: "press", key, sessionId: session.id, tabIndex };
			});
		},
	},
	"wait-for": {
		summary: "Wait for a selector to reach a given state.",
		usage: [
			"bun run driver -- wait-for --selector testid:dashboard-action-open-project",
			"bun run driver -- wait-for --selector css:.toast --state hidden",
		],
		examples: [
			"bun run driver -- wait-for --selector testid:setup-status-gcloud",
			"bun run driver -- wait-for --selector text:Saved --state visible",
		],
		async handler(args) {
			return withConnectedSession(args, async ({ page, session, tabIndex }) => {
				const selector = requireStringFlag(args, "selector");
				const locator = resolveLocator(page, selector);
				const state: "attached" | "detached" | "hidden" | "visible" =
					typeof flag(args, "state") === "string"
						? (flag(args, "state") as
								| "attached"
								| "detached"
								| "hidden"
								| "visible")
						: "visible";
				await locator.waitFor({
					state,
					timeout: optionalNumberFlag(args, "timeout"),
				});
				return {
					action: "wait-for",
					sessionId: session.id,
					state,
					tabIndex,
					...locatorSummary(selector),
				};
			});
		},
	},
	eval: {
		summary: "Evaluate a JavaScript expression in the active page.",
		usage: [
			"bun run driver -- eval --js 'document.title'",
			"bun run driver -- eval --tab 1 --js 'location.href'",
		],
		examples: [
			"bun run driver -- eval --js 'window.location.href'",
			"bun run driver -- eval --js 'document.querySelectorAll(\"button\").length'",
		],
		async handler(args) {
			return withConnectedSession(args, async ({ page, session, tabIndex }) => {
				const js = requireStringFlag(args, "js");
				return {
					result: await page.evaluate((expression) => {
						// biome-ignore lint/security/noGlobalEval: driver-only escape hatch
						return globalThis.eval(expression);
					}, js),
					sessionId: session.id,
					tabIndex,
				};
			});
		},
	},
	text: {
		summary: "Read the text content of the first matching element.",
		usage: [
			"bun run driver -- text --selector testid:setup-status-gcloud",
			"bun run driver -- text --selector css:.message --raw",
		],
		examples: [
			"bun run driver -- text --selector text:Google Cloud",
			"bun run driver -- text --selector css:main h1",
		],
		async handler(args) {
			return withConnectedSession(args, async ({ page, session, tabIndex }) => {
				const selector = requireStringFlag(args, "selector");
				const raw = booleanFlag(args, "raw");
				const text = await readLocatorText(
					resolveLocator(page, selector),
					optionalNumberFlag(args, "timeout"),
				);
				return {
					sessionId: session.id,
					tabIndex,
					text: trimText(text, raw),
					...locatorSummary(selector),
				};
			});
		},
	},
	html: {
		summary: "Read the outer HTML of the first matching element.",
		usage: [
			"bun run driver -- html --selector testid:setup-status-gcloud",
			"bun run driver -- html --selector css:main",
		],
		examples: [
			"bun run driver -- html --selector css:main",
			"bun run driver -- html --selector text:Open Project",
		],
		async handler(args) {
			return withConnectedSession(args, async ({ page, session, tabIndex }) => {
				const selector = requireStringFlag(args, "selector");
				const first = await firstMatch(
					resolveLocator(page, selector),
					optionalNumberFlag(args, "timeout"),
				);
				return {
					html: await first.evaluate((element) => element.outerHTML),
					sessionId: session.id,
					tabIndex,
					...locatorSummary(selector),
				};
			});
		},
	},
	attr: {
		summary: "Read an attribute from the first matching element.",
		usage: [
			"bun run driver -- attr --selector testid:setup-status-gcloud --name data-status-label",
		],
		examples: [
			"bun run driver -- attr --selector css:button --name disabled",
			"bun run driver -- attr --selector testid:setup-status-gcloud --name data-status-label",
		],
		async handler(args) {
			return withConnectedSession(args, async ({ page, session, tabIndex }) => {
				const selector = requireStringFlag(args, "selector");
				const name = requireStringFlag(args, "name");
				const first = await firstMatch(
					resolveLocator(page, selector),
					optionalNumberFlag(args, "timeout"),
				);
				return {
					name,
					sessionId: session.id,
					tabIndex,
					value: await first.getAttribute(name),
					...locatorSummary(selector),
				};
			});
		},
	},
	exists: {
		summary: "Check whether a selector exists, optionally requiring visibility.",
		usage: [
			"bun run driver -- exists --selector testid:dashboard-action-open-project",
			"bun run driver -- exists --selector css:.toast --visible",
		],
		examples: [
			"bun run driver -- exists --selector testid:setup-status-github",
			"bun run driver -- exists --selector css:dialog --visible",
		],
		async handler(args) {
			return withConnectedSession(args, async ({ page, session, tabIndex }) => {
				const selector = requireStringFlag(args, "selector");
				const locator = resolveLocator(page, selector);
				const count = await locator.count();
				const exists = booleanFlag(args, "visible")
					? await locator.first().isVisible().catch(() => false)
					: count > 0;
				return {
					count,
					exists,
					sessionId: session.id,
					tabIndex,
					visibleOnly: booleanFlag(args, "visible"),
					...locatorSummary(selector),
				};
			});
		},
	},
	count: {
		summary: "Count matching elements.",
		usage: [
			"bun run driver -- count --selector css:button",
			"bun run driver -- count --selector testid:setup-status-gcloud",
		],
		examples: [
			"bun run driver -- count --selector css:[data-testid]",
			"bun run driver -- count --selector role:button",
		],
		async handler(args) {
			return withConnectedSession(args, async ({ page, session, tabIndex }) => {
				const selector = requireStringFlag(args, "selector");
				return {
					count: await resolveLocator(page, selector).count(),
					sessionId: session.id,
					tabIndex,
					...locatorSummary(selector),
				};
			});
		},
	},
	assert: {
		summary: "Assert a selector state such as visible, hidden, enabled, or disabled.",
		usage: [
			"bun run driver -- assert --selector testid:dashboard-action-open-project --visible",
			"bun run driver -- assert --selector css:button --disabled",
		],
		examples: [
			"bun run driver -- assert --selector text:Open Project --visible",
			"bun run driver -- assert --selector css:.toast --hidden",
		],
		async handler(args) {
			return withConnectedSession(args, async ({ page, session, tabIndex }) => {
				const selector = requireStringFlag(args, "selector");
				const timeout = optionalNumberFlag(args, "timeout") ?? 5_000;
				const locator = resolveLocator(page, selector);
				const checks = [
					["visible", booleanFlag(args, "visible")],
					["hidden", booleanFlag(args, "hidden")],
					["attached", booleanFlag(args, "attached")],
					["detached", booleanFlag(args, "detached")],
					["enabled", booleanFlag(args, "enabled")],
					["disabled", booleanFlag(args, "disabled")],
				].filter(([, enabled]) => enabled);

				if (checks.length > 1) {
					throw new CliError("Use only one assert state flag at a time.", {
						command: "assert",
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
							() => locator.first().isEnabled().catch(() => false),
							`${selector} to become enabled`,
							timeout,
						);
						break;
					case "disabled":
						await firstMatch(locator, timeout);
						await assertBooleanState(
							() => locator.first().isDisabled().catch(() => false),
							`${selector} to become disabled`,
							timeout,
						);
						break;
					default:
						throw new CliError(`Unsupported assert state: ${state}`, {
							command: "assert",
						});
				}

				return {
					asserted: state,
					sessionId: session.id,
					tabIndex,
					...locatorSummary(selector),
				};
			});
		},
	},
	"assert-text": {
		summary: "Assert text equality or containment on the first matching element.",
		usage: [
			"bun run driver -- assert-text --selector testid:setup-status-gcloud --contains connected",
			"bun run driver -- assert-text --selector css:h1 --equals Dashboard",
		],
		examples: [
			"bun run driver -- assert-text --selector css:h1 --equals Silo",
			"bun run driver -- assert-text --selector testid:setup-status-github --contains GitHub",
		],
		async handler(args) {
			return withConnectedSession(args, async ({ page, session, tabIndex }) => {
				const selector = requireStringFlag(args, "selector");
				const equals =
					typeof flag(args, "equals") === "string"
						? (flag(args, "equals") as string)
						: undefined;
				const contains =
					typeof flag(args, "contains") === "string"
						? (flag(args, "contains") as string)
						: undefined;
				if (!equals && !contains) {
					throw new CliError("Provide either --equals or --contains.", {
						command: "assert-text",
						hint: "Example: bun run driver -- assert-text --selector css:h1 --equals Silo",
					});
				}
				if (equals && contains) {
					throw new CliError("Use only one of --equals or --contains.", {
						command: "assert-text",
					});
				}

				const actual = trimText(
					await readLocatorText(
						resolveLocator(page, selector),
						optionalNumberFlag(args, "timeout") ?? 5_000,
					),
					booleanFlag(args, "raw"),
				);

				if (typeof equals === "string" && actual !== equals) {
					throw new CliError(
						`Text assertion failed for ${selector}: expected exactly "${equals}", got "${actual}".`,
						{ command: "assert-text" },
					);
				}
				if (typeof contains === "string" && !actual.includes(contains)) {
					throw new CliError(
						`Text assertion failed for ${selector}: expected text containing "${contains}", got "${actual}".`,
						{ command: "assert-text" },
					);
				}

				return {
					actual,
					asserted: equals ? "equals" : "contains",
					expected: equals ?? contains,
					sessionId: session.id,
					tabIndex,
					...locatorSummary(selector),
				};
			});
		},
	},
	"assert-attr": {
		summary: "Assert an attribute exists, equals a value, or contains a value.",
		usage: [
			"bun run driver -- assert-attr --selector testid:setup-status-gcloud --name data-status-label --contains connected",
		],
		examples: [
			"bun run driver -- assert-attr --selector css:button --name disabled --exists",
			"bun run driver -- assert-attr --selector testid:setup-status-gcloud --name data-status-label --contains Google Cloud",
		],
		async handler(args) {
			return withConnectedSession(args, async ({ page, session, tabIndex }) => {
				const selector = requireStringFlag(args, "selector");
				const name = requireStringFlag(args, "name");
				const equals =
					typeof flag(args, "equals") === "string"
						? (flag(args, "equals") as string)
						: undefined;
				const contains =
					typeof flag(args, "contains") === "string"
						? (flag(args, "contains") as string)
						: undefined;
				const exists = booleanFlag(args, "exists");
				const checks = [Boolean(equals), Boolean(contains), exists].filter(Boolean).length;
				if (checks !== 1) {
					throw new CliError(
						"Choose exactly one attribute assertion: --exists, --equals, or --contains.",
						{ command: "assert-attr" },
					);
				}

				const first = await firstMatch(
					resolveLocator(page, selector),
					optionalNumberFlag(args, "timeout") ?? 5_000,
				);
				const value = await first.getAttribute(name);

				if (exists && value === null) {
					throw new CliError(
						`Attribute assertion failed for ${selector}: expected attribute "${name}" to exist.`,
						{ command: "assert-attr" },
					);
				}
				if (typeof equals === "string" && value !== equals) {
					throw new CliError(
						`Attribute assertion failed for ${selector}: expected ${name}="${equals}", got ${value === null ? "null" : `"${value}"`}.`,
						{ command: "assert-attr" },
					);
				}
				if (typeof contains === "string" && !(value ?? "").includes(contains)) {
					throw new CliError(
						`Attribute assertion failed for ${selector}: expected ${name} to contain "${contains}", got ${value === null ? "null" : `"${value}"`}.`,
						{ command: "assert-attr" },
					);
				}

				return {
					actual: value,
					asserted: exists ? "exists" : equals ? "equals" : "contains",
					expected: equals ?? contains ?? true,
					name,
					sessionId: session.id,
					tabIndex,
					...locatorSummary(selector),
				};
			});
		},
	},
	"tabs.list": {
		summary: "List open CDP pages for the current session.",
		usage: [
			"bun run driver -- tabs.list",
			"bun run driver -- tabs.list --session <id>",
		],
		examples: [
			"bun run driver -- tabs.list",
			"bun run driver -- tabs.list --session 20260319-example",
		],
		async handler(args) {
			const session = resolveSessionRecord(
				typeof flag(args, "session") === "string"
					? (flag(args, "session") as string)
					: undefined,
			);
			let connected: Awaited<ReturnType<typeof connectToDriverSession>>;
			try {
				connected = await connectToDriverSession(session);
			} catch (error) {
				throw attachFailure("tabs.list", session, error);
			}
			try {
				const mainPage = connected.page;
				const tabs = await Promise.all(
					connected.context.pages().map(async (page, index) => ({
						index,
						isDefault: page === mainPage,
						isMain:
							page.url().startsWith("http://tauri.localhost/") ||
							page.url().startsWith("http://localhost:3000"),
						title: await page.title().catch(() => ""),
						url: page.url(),
					})),
				);
				return { sessionId: session.id, tabs };
			} finally {
				await disconnectFromDriverSession(connected);
			}
		},
	},
	"video.status": {
		summary: "Report the current session's trace video paths and file status.",
		usage: [
			"bun run driver -- video.status",
			"bun run driver -- video.status --session <id>",
		],
		examples: [
			"bun run driver -- video.status",
			"bun run driver -- video.status --session trace-smoke",
			"bun run driver -- video.status --trace-id trace-smoke",
		],
		async handler(args) {
			const sessionId =
				typeof flag(args, "session") === "string"
					? (flag(args, "session") as string)
					: undefined;
			const traceId =
				typeof flag(args, "trace-id") === "string"
					? (flag(args, "trace-id") as string)
					: undefined;
			const sourceStateDir =
				typeof flag(args, "source-state-dir") === "string"
					? (flag(args, "source-state-dir") as string)
					: defaultSourceStateDir;
			const session = sessionId ? resolveSessionRecord(sessionId) : undefined;
			const resolvedTraceId = traceId ?? session?.traceId;
			if (!resolvedTraceId) {
				throw new CliError("Provide either --session or --trace-id.", {
					command: "video.status",
				});
			}

			const traceDir = session?.traceDir ?? traceDirFor(sourceStateDir, resolvedTraceId);
			const videoMetadataPath = session?.videoMetadataPath
				?? path.join(traceDir, "video-metadata.json");
			const videoPath = session?.videoPath ?? path.join(traceDir, "video.mp4");
			const metadataExists = existsSync(videoMetadataPath);
			const videoExists = existsSync(videoPath);
			const metadata = metadataExists
				? JSON.parse(readFileSync(videoMetadataPath, "utf8"))
				: null;
			return {
				sessionId: session?.id ?? null,
				traceId: resolvedTraceId,
				videoExists,
				videoMetadataExists: metadataExists,
				videoMetadataPath,
				videoPath,
				videoRecorderPid: session?.videoRecorderPid ?? null,
				metadata,
			};
		},
	},
	"app.wait-ready": {
		summary: "Wait for the Silo dashboard and service statuses to become ready.",
		usage: [
			"bun run driver -- app.wait-ready",
			"bun run driver -- app.wait-ready --timeout 120000",
		],
		examples: [
			"bun run driver -- app.wait-ready",
			"bun run driver -- app.wait-ready --session <id>",
		],
		async handler(args) {
			return withConnectedSession(args, async ({ page, session, tabIndex }) => ({
				sessionId: session.id,
				status: await waitForAppReady(
					page,
					optionalNumberFlag(args, "timeout") ?? 60_000,
				),
				tabIndex,
			}));
		},
	},
	"app.status": {
		summary: "Return high-level app status for the active page.",
		usage: [
			"bun run driver -- app.status",
			"bun run driver -- app.status --session <id>",
		],
		examples: [
			"bun run driver -- app.status",
			"bun run driver -- app.status --tab 0",
		],
		async handler(args) {
			return withConnectedSession(args, async ({ page, session, tabIndex }) => ({
				sessionId: session.id,
				status: await readAppStatus(page),
				tabIndex,
			}));
		},
	},
	"app.service-status": {
		summary: "Return service connectivity labels for the dashboard, optionally narrowed to one service.",
		usage: [
			"bun run driver -- app.service-status",
			"bun run driver -- app.service-status --service gcloud",
		],
		examples: [
			"bun run driver -- app.service-status --service github",
			"bun run driver -- app.service-status",
		],
		async handler(args) {
			return withConnectedSession(args, async ({ page, session, tabIndex }) => {
				const services = await readAppServiceStatuses(page);
				const service =
					typeof flag(args, "service") === "string"
						? (flag(args, "service") as string)
						: undefined;
				if (!service) {
					return {
						services,
						sessionId: session.id,
						tabIndex,
					};
				}

				if (!(service in services)) {
					throw new CliError(`Unknown service: ${service}`, {
						command: "app.service-status",
						hint: "Use one of: gcloud, github, codex, claude.",
					});
				}

				return {
					service,
					sessionId: session.id,
					tabIndex,
					value: services[service as keyof typeof services],
				};
			});
		},
	},
};

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
		try {
			process.stderr.write(`${commandHelpLines(cliError.command)}\n\n`);
		} catch {}
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
				usage: cliError.usage ?? (cliError.command ? commandDefinitions[cliError.command]?.usage ?? null : null),
			},
			null,
			2,
		)}\n`,
	);
}

async function main() {
	const argv = process.argv.slice(2);
	const startedAt = new Date().toISOString();
	const startedAtMs = Date.now();
	const args = parseCommand(argv);
	const session = resolveLoggingSession(args);

	try {
		assertKnownCommand(args.command);
		const handler = commandDefinitions[args.command].handler;
		const result = await handler(args);
		printJson(result);
		logCommandAttempt(args, argv, startedAt, startedAtMs, {
			error: null,
			result,
			session,
		});
	} catch (error) {
		const errorSession =
			error instanceof DriverLaunchError ? error.session : session;
		printError(error);
		logCommandAttempt(args, argv, startedAt, startedAtMs, {
			error: error instanceof Error ? error.message : String(error),
			session: errorSession,
		});
		process.exitCode = 1;
	}
}

void main();
