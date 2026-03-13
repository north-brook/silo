import type { Terminal } from "@xterm/xterm";

const COMMAND_LEFT = "\u0001";
const COMMAND_RIGHT = "\u0005";
const COMMAND_BACKSPACE = "\u0015";
const ALT_LEFT = "\u001bb";
const ALT_RIGHT = "\u001bf";

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

function sequenceForEvent(event: KeyboardEvent): string | null {
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
