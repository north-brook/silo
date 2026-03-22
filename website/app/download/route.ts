import { NextRequest, NextResponse } from "next/server";
import {
	extractAssetName,
	fetchLatestReleaseManifest,
	isSafeAssetName,
	isSafeVersion,
	normalizeVersion,
	preferredInstallerAssetName,
	releasePlatformKey,
	versionedAssetDownloadUrl,
} from "../../lib/github-releases";

export const runtime = "nodejs";

export async function GET(request: NextRequest) {
	const version = request.nextUrl.searchParams.get("version")?.trim();
	const asset = request.nextUrl.searchParams.get("asset")?.trim();

	if ((version && !asset) || (!version && asset)) {
		return NextResponse.json(
			{ error: "version and asset must be provided together" },
			{ status: 400 },
		);
	}

	if (version && asset) {
		if (!isSafeVersion(version) || !isSafeAssetName(asset)) {
			return NextResponse.json(
				{ error: "version or asset is invalid" },
				{ status: 400 },
			);
		}

		return NextResponse.redirect(versionedAssetDownloadUrl(version, asset), {
			status: 307,
			headers: {
				"Cache-Control": "public, max-age=3600, s-maxage=3600",
			},
		});
	}

	try {
		const manifest = await fetchLatestReleaseManifest();
		const latestVersion = normalizeVersion(manifest.version);
		const preferredAssetName = preferredInstallerAssetName();

		if (preferredAssetName) {
			return NextResponse.redirect(
				versionedAssetDownloadUrl(latestVersion, preferredAssetName),
				{
					status: 307,
					headers: {
						"Cache-Control":
							"public, max-age=300, s-maxage=300, stale-while-revalidate=60",
					},
				},
			);
		}

		const defaultMacPlatform = manifest.platforms?.[releasePlatformKey("darwin", "aarch64")];
		if (!defaultMacPlatform?.url) {
			return NextResponse.json(
				{ error: "no default macOS download artifact is available" },
				{ status: 502 },
			);
		}

		return NextResponse.redirect(
			versionedAssetDownloadUrl(latestVersion, extractAssetName(defaultMacPlatform.url)),
			{
				status: 307,
				headers: {
					"Cache-Control": "public, max-age=300, s-maxage=300, stale-while-revalidate=60",
				},
			},
		);
	} catch (error) {
		const message =
			error instanceof Error ? error.message : "failed to resolve latest release metadata";
		return NextResponse.json({ error: message }, { status: 502 });
	}
}
