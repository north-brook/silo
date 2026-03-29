import { describe, expect, test } from "bun:test";
import { InvokeResultCache } from "./invoke-cache";

describe("InvokeResultCache", () => {
	test("returns cached values", () => {
		let now = 1000;
		const cache = new InvokeResultCache<string>({
			maxEntries: 2,
			now: () => now,
			ttlMs: 1000,
		});
		cache.set("a", "first");
		expect(cache.get("a")).toBe("first");
		expect(cache.size).toBe(1);
	});

	test("evicts the least recently used entry when full", () => {
		let now = 1000;
		const cache = new InvokeResultCache<string>({
			maxEntries: 2,
			now: () => now,
			ttlMs: 10_000,
		});
		cache.set("a", "first");
		now += 1;
		cache.set("b", "second");
		now += 1;
		cache.get("a");
		now += 1;
		cache.set("c", "third");

		expect(cache.get("a")).toBe("first");
		expect(cache.get("b")).toBeUndefined();
		expect(cache.get("c")).toBe("third");
		expect(cache.size).toBe(2);
	});

	test("expires stale entries", () => {
		let now = 1000;
		const cache = new InvokeResultCache<string>({
			maxEntries: 2,
			now: () => now,
			ttlMs: 100,
		});
		cache.set("a", "first");
		now += 200;
		expect(cache.get("a")).toBeUndefined();
		expect(cache.size).toBe(0);
	});
});
