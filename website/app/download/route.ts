import { NextRequest, NextResponse } from "next/server";
import {
	isSafeAssetName,
	isSafeVersion,
	latestInstallerDownloadUrl,
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

	return NextResponse.redirect(latestInstallerDownloadUrl(), {
		status: 307,
		headers: {
			"Cache-Control": "public, max-age=300, s-maxage=300, stale-while-revalidate=60",
		},
	});
}
