import { describe, expect, test } from "bun:test";
import { parseRunningSiloApps } from "./processes";

describe("parseRunningSiloApps", () => {
	test("matches only the dev app for driver launches", () => {
		const output = [
			"101 /Applications/Silo.app/Contents/MacOS/Silo",
			"202 /private/tmp/Silo Dev.app/Contents/MacOS/Silo Dev",
		].join("\n");

		expect(parseRunningSiloApps(output, "dev")).toEqual([
			{
				command: "/private/tmp/Silo Dev.app/Contents/MacOS/Silo Dev",
				pid: 202,
			},
		]);
	});

	test("can still identify production app processes separately", () => {
		const output = [
			"101 /Applications/Silo.app/Contents/MacOS/Silo",
			"202 /private/tmp/Silo Dev.app/Contents/MacOS/Silo Dev",
		].join("\n");

		expect(parseRunningSiloApps(output, "prod")).toEqual([
			{
				command: "/Applications/Silo.app/Contents/MacOS/Silo",
				pid: 101,
			},
		]);
	});

	test("ignores playwright helper processes", () => {
		const output =
			"303 /private/tmp/Silo Dev.app/Contents/MacOS/Silo Dev -- launched by playwright test";

		expect(parseRunningSiloApps(output, "dev")).toEqual([]);
	});
});
