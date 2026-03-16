import type { Terminal } from "@xterm/xterm";

const COMMAND_LEFT = "\u0001";
const COMMAND_RIGHT = "\u0005";
const COMMAND_BACKSPACE = "\u0015";
const ALT_LEFT = "\u001bb";
const ALT_RIGHT = "\u001bf";
const SHIFT_ENTER = "\n";

interface TerminalBindingEvent {
	altKey: boolean;
	code?: string;
	ctrlKey: boolean;
	key: string;
	metaKey: boolean;
	shiftKey: boolean;
}

function matchesKey(event: TerminalBindingEvent, expected: string) {
	return event.key === expected || event.code === expected;
}

export function attachTerminalBindings(
	term: Terminal,
	sendData: (data: string) => void,
) {
	term.attachCustomKeyEventHandler((event) => {
		if (event.type !== "keydown") {
			return true;
		}

		const sequence = sequenceForEvent(event);
		if (!sequence) {
			return true;
		}

		event.preventDefault();
		sendData(sequence);
		return false;
	});

	return () => {
		term.attachCustomKeyEventHandler(() => true);
	};
}

export function sequenceForEvent(event: TerminalBindingEvent): string | null {
	if (
		event.shiftKey &&
		!event.metaKey &&
		!event.altKey &&
		!event.ctrlKey &&
		matchesKey(event, "Enter")
	) {
		return SHIFT_ENTER;
	}

	if (event.metaKey && !event.altKey && !event.ctrlKey) {
		if (matchesKey(event, "ArrowLeft")) {
			return COMMAND_LEFT;
		}
		if (matchesKey(event, "ArrowRight")) {
			return COMMAND_RIGHT;
		}
		if (matchesKey(event, "Backspace")) {
			return COMMAND_BACKSPACE;
		}
		return null;
	}

	if (event.altKey && !event.metaKey && !event.ctrlKey) {
		if (matchesKey(event, "ArrowLeft")) {
			return ALT_LEFT;
		}
		if (matchesKey(event, "ArrowRight")) {
			return ALT_RIGHT;
		}
		return null;
	}

	return null;
}
