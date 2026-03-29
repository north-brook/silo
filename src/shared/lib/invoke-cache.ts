type CacheEntry<T> = {
	expiresAt: number;
	lastAccessedAt: number;
	value: T;
};

export class InvokeResultCache<T> {
	private entries = new Map<string, CacheEntry<T>>();

	constructor(
		private readonly options: {
			maxEntries: number;
			now?: () => number;
			ttlMs: number;
		},
	) {}

	get size() {
		this.pruneExpired();
		return this.entries.size;
	}

	get(key: string) {
		this.pruneExpired();
		const entry = this.entries.get(key);
		if (!entry) {
			return undefined;
		}

		entry.lastAccessedAt = this.now();
		return entry.value;
	}

	set(key: string, value: T) {
		const now = this.now();
		this.pruneExpired(now);
		this.entries.set(key, {
			expiresAt: now + this.options.ttlMs,
			lastAccessedAt: now,
			value,
		});
		this.pruneOverflow();
	}

	private now() {
		return this.options.now?.() ?? Date.now();
	}

	private pruneExpired(now = this.now()) {
		for (const [key, entry] of this.entries) {
			if (entry.expiresAt <= now) {
				this.entries.delete(key);
			}
		}
	}

	private pruneOverflow() {
		if (this.entries.size <= this.options.maxEntries) {
			return;
		}

		const victims = [...this.entries.entries()]
			.sort((left, right) => left[1].lastAccessedAt - right[1].lastAccessedAt)
			.slice(0, this.entries.size - this.options.maxEntries);
		for (const [key] of victims) {
			this.entries.delete(key);
		}
	}
}
