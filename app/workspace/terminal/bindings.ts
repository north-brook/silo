import type { Terminal } from "@xterm/xterm";

const COMMAND_LEFT = "\u0001";
const COMMAND_RIGHT = "\u0005";
const COMMAND_BACKSPACE = "\u001b[1337;1u";
const ALT_LEFT = "\u001bb";
const ALT_RIGHT = "\u001bf";
const SHIFT_ENTER = "\n";

interface TerminalBindingEvent {
	altKey: boolean;
	ctrlKey: boolean;
	key: string;
	metaKey: boolean;
	shiftKey: boolean;
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
		event.key === "Enter"
	) {
		return SHIFT_ENTER;
	}

	if (event.metaKey && !event.altKey && !event.ctrlKey) {
		switch (event.key) {
			case "ArrowLeft":
				return COMMAND_LEFT;
			case "ArrowRight":
				return COMMAND_RIGHT;
			case "Backspace":
				return COMMAND_BACKSPACE;
			default:
				return null;
		}
	}

	if (event.altKey && !event.metaKey && !event.ctrlKey) {
		switch (event.key) {
			case "ArrowLeft":
				return ALT_LEFT;
			case "ArrowRight":
				return ALT_RIGHT;
			default:
				return null;
		}
	}

	return null;
}
