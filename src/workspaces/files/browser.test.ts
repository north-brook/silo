import { describe, expect, test } from "bun:test";
import { filePathOpensInBrowser } from "./browser";

describe("filePathOpensInBrowser", () => {
	test("matches supported browser-renderable file extensions", () => {
		expect(filePathOpensInBrowser("images/mockup.PNG")).toBe(true);
		expect(filePathOpensInBrowser("docs/spec.final.pdf")).toBe(true);
		expect(filePathOpensInBrowser("icons/logo.svg")).toBe(true);
	});

	test("rejects unsupported or extensionless paths", () => {
		expect(filePathOpensInBrowser("archive.zip")).toBe(false);
		expect(filePathOpensInBrowser("README")).toBe(false);
	});
});
