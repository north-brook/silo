import { NextRequest, NextResponse } from "next/server";
import {
	compareVersions,
	extractAssetName,
	fetchLatestReleaseManifest,
	normalizeVersion,
	releasePlatformKey,
} from "../../lib/github-releases";

export const runtime = "nodejs";

export async function GET(request: NextRequest) {
	const target = request.nextUrl.searchParams.get("target")?.trim();
	const arch = request.nextUrl.searchParams.get("arch")?.trim();
	const currentVersion = request.nextUrl.searchParams.get("current_version")?.trim();

	if (!target || !arch || !currentVersion) {
		return NextResponse.json(
			{ error: "target, arch, and current_version are required" },
			{ status: 400 },
		);
	}

	try {
		const manifest = await fetchLatestReleaseManifest();
		const latestVersion = normalizeVersion(manifest.version);

		if (compareVersions(currentVersion, latestVersion) >= 0) {
			return new NextResponse(null, {
				status: 204,
				headers: {
					"Cache-Control": "public, max-age=300, s-maxage=300, stale-while-revalidate=60",
				},
			});
		}

		const platform = manifest.platforms?.[releasePlatformKey(target, arch)];
		if (!platform?.url || !platform.signature) {
			return NextResponse.json(
				{ error: `no updater artifact for ${releasePlatformKey(target, arch)}` },
				{ status: 404 },
			);
		}

		const downloadUrl = new URL("/download", request.url);
		downloadUrl.searchParams.set("version", latestVersion);
		downloadUrl.searchParams.set("asset", extractAssetName(platform.url));

		return NextResponse.json(
			{
				version: latestVersion,
				pub_date: manifest.pub_date,
				url: downloadUrl.toString(),
				signature: platform.signature,
				notes: manifest.notes,
			},
			{
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
