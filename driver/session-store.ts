import {
	readdirSync,
	readFileSync,
	rmSync,
	statSync,
	writeFileSync,
} from "node:fs";
import path from "node:path";
import { driverSessionDir } from "./paths";
import type { DriverSessionRecord } from "./types";
import { ensureDirectory } from "./utils";

function sessionPath(id: string) {
	return path.join(driverSessionDir, `${id}.json`);
}

export function writeSessionRecord(session: DriverSessionRecord) {
	ensureDirectory(driverSessionDir);
	writeFileSync(
		sessionPath(session.id),
		`${JSON.stringify(session, null, 2)}\n`,
	);
	return sessionPath(session.id);
}

export function readSessionRecord(id: string) {
	return JSON.parse(
		readFileSync(sessionPath(id), "utf8"),
	) as DriverSessionRecord;
}

export function removeSessionRecord(id: string) {
	rmSync(sessionPath(id), { force: true });
}

export function listSessionRecords() {
	ensureDirectory(driverSessionDir);

	return readdirSync(driverSessionDir)
		.filter((entry) => entry.endsWith(".json"))
		.map((entry) => {
			const pathname = path.join(driverSessionDir, entry);
			const stats = statSync(pathname);
			const session = JSON.parse(
				readFileSync(pathname, "utf8"),
			) as DriverSessionRecord;
			return { session, timestamp: stats.mtimeMs };
		})
		.sort((left, right) => right.timestamp - left.timestamp)
		.map(({ session }) => session);
}

export function latestSessionRecord() {
	const [latest] = listSessionRecords();
	if (!latest) {
		throw new Error(
			"No driver sessions found. Run `bun run driver -- session launch` first.",
		);
	}

	return latest;
}

export function resolveSessionRecord(id: string) {
	return id === "latest" ? latestSessionRecord() : readSessionRecord(id);
}
