const RETRYABLE_TERMINAL_TRANSPORT_PATTERNS = [
	"can't assign requested address",
	"broken pipe",
	"connection refused",
	"connection reset",
	"connection reset by peer",
	"connection timed out",
	"connection closed",
	"connection lost",
	"network is unreachable",
	"operation timed out",
	"port 22",
	"software caused connection abort",
	"timed out",
	"transport endpoint is not connected",
];

const TERMINAL_RECONNECT_DELAYS_MS = [1000, 2000, 5000, 10000, 15000];

export function reconnectDelayMs(attempt: number) {
	if (!Number.isFinite(attempt) || attempt <= 0) {
		return TERMINAL_RECONNECT_DELAYS_MS[0];
	}

	const index = Math.min(attempt - 1, TERMINAL_RECONNECT_DELAYS_MS.length - 1);
	return TERMINAL_RECONNECT_DELAYS_MS[index];
}

export function isRetryableTerminalTransportMessage(message: string) {
	const lower = message.toLowerCase();
	return RETRYABLE_TERMINAL_TRANSPORT_PATTERNS.some((pattern) =>
		lower.includes(pattern),
	);
}
