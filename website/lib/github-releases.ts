const DEFAULT_GITHUB_REPOSITORY = "north-brook/silo";
const DEFAULT_INSTALLER_ASSET_NAME = "Silo-macos-arm64.dmg";
const USER_AGENT = "silo.new";

type ReleasePlatform = {
	signature?: string;
	url?: string;
};

type LatestReleaseManifest = {
	notes?: string;
	platforms?: Record<string, ReleasePlatform>;
	pub_date?: string;
	version: string;
};

function envValue(name: string): string | undefined {
	const value = process.env[name]?.trim();
	return value ? value : undefined;
}

export function githubRepository(): string {
	return envValue("SILO_RELEASE_GITHUB_REPOSITORY") ?? DEFAULT_GITHUB_REPOSITORY;
}

export function githubRepositoryUrl(): string {
	return `https://github.com/${githubRepository()}`;
}

export function latestInstallerAssetName(): string {
	return envValue("SILO_RELEASE_INSTALLER_ASSET_NAME") ?? DEFAULT_INSTALLER_ASSET_NAME;
}

function latestReleaseManifestUrl(): string {
	return `${githubRepositoryUrl()}/releases/latest/download/latest.json`;
}

export function latestInstallerDownloadUrl(): string {
	return `${githubRepositoryUrl()}/releases/latest/download/${encodeURIComponent(latestInstallerAssetName())}`;
}

export async function fetchLatestReleaseManifest(): Promise<LatestReleaseManifest> {
	const response = await fetch(latestReleaseManifestUrl(), {
		headers: {
			accept: "application/json",
			"user-agent": USER_AGENT,
		},
		next: { revalidate: 300 },
	});

	if (!response.ok) {
		throw new Error(
			`failed to fetch latest release manifest (${response.status} ${response.statusText})`,
		);
	}

	const data = (await response.json()) as unknown;
	if (!isLatestReleaseManifest(data)) {
		throw new Error("latest release manifest is missing required fields");
	}

	return data;
}

export function releasePlatformKey(target: string, arch: string): string {
	return `${target}-${arch}`;
}

export function extractAssetName(downloadUrl: string): string {
	const assetName = decodeURIComponent(new URL(downloadUrl).pathname.split("/").pop() ?? "");
	if (!isSafeAssetName(assetName)) {
		throw new Error("release asset name is invalid");
	}
	return assetName;
}

export function versionedAssetDownloadUrl(version: string, assetName: string): string {
	if (!isSafeVersion(version)) {
		throw new Error("release version is invalid");
	}
	if (!isSafeAssetName(assetName)) {
		throw new Error("release asset name is invalid");
	}

	return `${githubRepositoryUrl()}/releases/download/v${normalizeVersion(version)}/${encodeURIComponent(assetName)}`;
}

export function normalizeVersion(version: string): string {
	return version.trim().replace(/^v/i, "");
}

export function compareVersions(a: string, b: string): number {
	const parsedA = parseVersion(a);
	const parsedB = parseVersion(b);

	if (parsedA && parsedB) {
		if (parsedA.major !== parsedB.major) {
			return parsedA.major - parsedB.major;
		}
		if (parsedA.minor !== parsedB.minor) {
			return parsedA.minor - parsedB.minor;
		}
		if (parsedA.patch !== parsedB.patch) {
			return parsedA.patch - parsedB.patch;
		}
		return 0;
	}

	return normalizeVersion(a).localeCompare(normalizeVersion(b), undefined, {
		numeric: true,
		sensitivity: "base",
	});
}

export function isSafeVersion(version: string): boolean {
	return /^[0-9A-Za-z._-]+$/.test(normalizeVersion(version));
}

export function isSafeAssetName(assetName: string): boolean {
	return (
		assetName.length > 0 &&
		!assetName.includes("/") &&
		!assetName.includes("\\") &&
		!assetName.includes("..")
	);
}

function parseVersion(version: string) {
	const match = /^v?(\d+)\.(\d+)\.(\d+)(?:[-+].*)?$/.exec(version.trim());
	if (!match) {
		return null;
	}

	return {
		major: Number(match[1]),
		minor: Number(match[2]),
		patch: Number(match[3]),
	};
}

function isLatestReleaseManifest(value: unknown): value is LatestReleaseManifest {
	if (!value || typeof value !== "object") {
		return false;
	}

	const record = value as Record<string, unknown>;
	return (
		typeof record.version === "string" &&
		(record.notes === undefined || typeof record.notes === "string") &&
		(record.pub_date === undefined || typeof record.pub_date === "string") &&
		(record.platforms === undefined || isPlatformRecord(record.platforms))
	);
}

function isPlatformRecord(value: unknown): value is Record<string, ReleasePlatform> {
	if (!value || typeof value !== "object") {
		return false;
	}

	return Object.values(value).every((entry) => {
		if (!entry || typeof entry !== "object") {
			return false;
		}

		const record = entry as Record<string, unknown>;
		return (
			(record.signature === undefined || typeof record.signature === "string") &&
			(record.url === undefined || typeof record.url === "string")
		);
	});
}
