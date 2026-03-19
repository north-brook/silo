import type { Browser, BrowserContext, Page } from "@playwright/test";

export type DriverSessionRecord = {
	id: string;
	createdAt: string;
	artifactsDir: string;
	stateDir: string;
	sourceStateDir: string;
	traceId: string;
	traceDir: string;
	manifestPath: string;
	driverLogPath: string;
	appLogPath: string;
	tauriStdoutPath: string;
	tauriStderrPath: string;
	viteStdoutPath: string | null;
	viteStderrPath: string | null;
	videoPath: string;
	videoMetadataPath: string;
	videoStdoutPath: string;
	videoStderrPath: string;
	cdpPort: number;
	cdpUrl: string;
	tauriPid: number;
	vitePid: number | null;
	videoRecorderPid: number | null;
	initialSiloPids: number[];
	platform: NodeJS.Platform;
};

export type ConnectedDriverSession = {
	session: DriverSessionRecord;
	browser: Browser;
	context: BrowserContext;
	page: Page;
};

export type LaunchedDriverSession = ConnectedDriverSession;

export type LaunchSessionOptions = {
	artifactsDir?: string;
	cdpPort?: number;
	id?: string;
	skipPreflight?: boolean;
	sourceStateDir?: string;
};

export type AppServiceStatus = {
	claude: string | null;
	codex: string | null;
	gcloud: string | null;
	github: string | null;
};

export type AppStatus = {
	openProjectVisible: boolean;
	pageTitle: string;
	pageUrl: string;
	services: AppServiceStatus;
};

export type ConsoleEntry = {
	level: string | null;
	message: string;
	raw: string;
	target: string | null;
	timestamp: string | null;
};

export type ParsedSelector =
	| { kind: "css"; value: string }
	| { kind: "label"; value: string }
	| { kind: "role"; role: string; name?: string }
	| { kind: "testid"; value: string }
	| { kind: "text"; value: string };
