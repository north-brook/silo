"use client";

import {
	createContext,
	useCallback,
	useContext,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { Loader } from "../loader";
import type { CloudSession } from "../../lib/cloud";
import { cloudSessionKey } from "../../lib/cloud";
import { CloudTerminalHost } from "./terminal";

type CloudHostStatus = "idle" | "attaching" | "ready" | "error";

interface CloudHostRecord {
	session: CloudSession;
	key: string;
	status: CloudHostStatus;
	errorMessage: string | null;
	terminalId: string | null;
	skipInitialScrollback: boolean;
}

interface CloudContextValue {
	activeSessionKey: string | null;
	activeWorkspace: string | null;
	ensureSession: (
		session: CloudSession,
		options?: { skipInitialScrollback?: boolean },
	) => void;
	ensureWorkspaceSessions: (workspace: string, sessions: CloudSession[]) => void;
	getHost: (key: string | null) => CloudHostRecord | null;
	registerWorkspaceOutlet: (
		workspace: string,
		element: HTMLDivElement | null,
	) => void;
	removeSession: (workspace: string, kind: string, attachmentId: string) => void;
	setActiveSession: (workspace: string | null, key: string | null) => void;
}

const CloudContext = createContext<CloudContextValue | null>(null);

function upsertHostRecord(
	existing: CloudHostRecord | undefined,
	session: CloudSession,
	options?: { skipInitialScrollback?: boolean },
): CloudHostRecord {
	return {
		key: existing?.key ?? cloudSessionKey(session),
		session,
		status: existing?.status ?? "idle",
		errorMessage: existing?.errorMessage ?? null,
		terminalId: existing?.terminalId ?? null,
		skipInitialScrollback:
			options?.skipInitialScrollback ??
			existing?.skipInitialScrollback ??
			false,
	};
}

function hiddenParkingLotClassName() {
	return "pointer-events-none fixed left-[-10000px] top-0 h-px w-px overflow-hidden opacity-0";
}

export function CloudProvider({
	children,
}: Readonly<{ children: React.ReactNode }>) {
	const [hosts, setHosts] = useState<Record<string, CloudHostRecord>>({});
	const [workspacePreloaded, setWorkspacePreloaded] = useState<
		Record<string, boolean>
	>({});
	const [activeWorkspace, setActiveWorkspace] = useState<string | null>(null);
	const [activeSessionKey, setActiveSessionKey] = useState<string | null>(null);
	const [workspaceOutlets, setWorkspaceOutlets] = useState<
		Record<string, HTMLDivElement | null>
	>({});
	const [parkingLotElement, setParkingLotElement] =
		useState<HTMLDivElement | null>(null);
	const parkingLotRef = useRef<HTMLDivElement>(null);
	const workspacePreloadedRef = useRef(workspacePreloaded);

	useEffect(() => {
		workspacePreloadedRef.current = workspacePreloaded;
	}, [workspacePreloaded]);

	useEffect(() => {
		setParkingLotElement(parkingLotRef.current);
	}, []);

	const ensureSession = useCallback(
		(session: CloudSession, options?: { skipInitialScrollback?: boolean }) => {
			setHosts((previous) => {
				const key = cloudSessionKey(session);
				const next = upsertHostRecord(previous[key], session, options);
				const existing = previous[key];
				if (
					existing &&
					existing.session.workspace === next.session.workspace &&
					existing.session.kind === next.session.kind &&
					existing.session.attachmentId === next.session.attachmentId &&
					existing.session.name === next.session.name &&
					existing.session.working === next.session.working &&
					existing.session.unread === next.session.unread &&
					existing.skipInitialScrollback === next.skipInitialScrollback
				) {
					return previous;
				}
				return {
					...previous,
					[key]: next,
				};
			});
		},
		[],
	);

	const ensureWorkspaceSessions = useCallback(
		(workspace: string, sessions: CloudSession[]) => {
			setHosts((previous) => {
				const preloaded = workspacePreloadedRef.current[workspace] === true;
				if (!preloaded) {
					let changed = false;
					const next = { ...previous };
					for (const session of sessions) {
						const key = cloudSessionKey(session);
						const nextRecord = upsertHostRecord(previous[key], session);
						const existing = previous[key];
						if (
							!existing ||
							existing.session.name !== nextRecord.session.name ||
							existing.session.working !== nextRecord.session.working ||
							existing.session.unread !== nextRecord.session.unread
						) {
							changed = true;
						}
						next[key] = nextRecord;
					}
					return changed ? next : previous;
				}

				let changed = false;
				const next = { ...previous };
				const nextKeys = new Set<string>();
				for (const session of sessions) {
					const key = cloudSessionKey(session);
					nextKeys.add(key);
					const nextRecord = upsertHostRecord(previous[key], session);
					const existing = previous[key];
					if (
						!existing ||
						existing.session.name !== nextRecord.session.name ||
						existing.session.working !== nextRecord.session.working ||
						existing.session.unread !== nextRecord.session.unread
					) {
						changed = true;
					}
					next[key] = nextRecord;
				}

				for (const [key, record] of Object.entries(previous)) {
					if (record.session.workspace !== workspace) {
						continue;
					}
					if (!nextKeys.has(key) && key !== activeSessionKey) {
						delete next[key];
						changed = true;
					}
				}

				return changed ? next : previous;
			});
			setWorkspacePreloaded((previous) => {
				if (previous[workspace]) {
					return previous;
				}
				return {
					...previous,
					[workspace]: true,
				};
			});
		},
		[activeSessionKey],
	);

	const removeSession = useCallback(
		(workspace: string, kind: string, attachmentId: string) => {
			const key = cloudSessionKey({ workspace, kind, attachmentId });
			setHosts((previous) => {
				if (!(key in previous)) {
					return previous;
				}

				const next = { ...previous };
				delete next[key];
				return next;
			});
		},
		[],
	);

	const registerWorkspaceOutlet = useCallback(
		(workspace: string, element: HTMLDivElement | null) => {
			setWorkspaceOutlets((previous) => {
				if (previous[workspace] === element) {
					return previous;
				}
				return {
					...previous,
					[workspace]: element,
				};
			});
		},
		[],
	);

	const setActiveSession = useCallback((workspace: string | null, key: string | null) => {
		setActiveWorkspace((previous) => (previous === workspace ? previous : workspace));
		setActiveSessionKey((previous) => (previous === key ? previous : key));
	}, []);

	const getHost = useCallback(
		(key: string | null) => {
			if (!key) {
				return null;
			}
			return hosts[key] ?? null;
		},
		[hosts],
	);

	const updateHost = useCallback(
		(
			key: string,
			state: {
				status: CloudHostStatus;
				errorMessage?: string | null;
				terminalId?: string | null;
			},
		) => {
			setHosts((previous) => {
				const existing = previous[key];
				if (!existing) {
					return previous;
				}

				const next = {
					...existing,
					status: state.status,
					errorMessage:
						state.errorMessage === undefined
							? existing.errorMessage
							: state.errorMessage,
					terminalId:
						state.terminalId === undefined
							? existing.terminalId
							: state.terminalId,
				};

				if (
					next.status === existing.status &&
					next.errorMessage === existing.errorMessage &&
					next.terminalId === existing.terminalId
				) {
					return previous;
				}

				return {
					...previous,
					[key]: next,
				};
			});
		},
		[],
	);

	const consumeFreshFlag = useCallback((key: string) => {
		setHosts((previous) => {
			const existing = previous[key];
			if (!existing || !existing.skipInitialScrollback) {
				return previous;
			}

			return {
				...previous,
				[key]: {
					...existing,
					skipInitialScrollback: false,
				},
			};
		});
	}, []);

	const contextValue = useMemo<CloudContextValue>(
		() => ({
			activeSessionKey,
			activeWorkspace,
			ensureSession,
			ensureWorkspaceSessions,
			getHost,
			registerWorkspaceOutlet,
			removeSession,
			setActiveSession,
		}),
		[
			activeSessionKey,
			activeWorkspace,
			ensureSession,
			ensureWorkspaceSessions,
			getHost,
			registerWorkspaceOutlet,
			removeSession,
			setActiveSession,
		],
	);

	return (
		<CloudContext.Provider value={contextValue}>
			{children}
			<div aria-hidden className={hiddenParkingLotClassName()}>
				<div ref={parkingLotRef} />
			</div>
			{parkingLotElement &&
				Object.values(hosts).map((record) => {
					const isActive =
						record.session.workspace === activeWorkspace &&
						record.key === activeSessionKey;
					const target = isActive
						? (workspaceOutlets[record.session.workspace] ?? parkingLotElement)
						: parkingLotElement;

					if (record.session.kind === "terminal") {
						return (
							<CloudTerminalHost
								key={record.key}
								session={record.session}
								target={target}
								visible={isActive}
								skipInitialScrollback={record.skipInitialScrollback}
								onFreshConsumed={() => consumeFreshFlag(record.key)}
								onHostStateChange={(state) => updateHost(record.key, state)}
							/>
						);
					}

					return null;
				})}
		</CloudContext.Provider>
	);
}

export function useCloud() {
	const context = useContext(CloudContext);
	if (!context) {
		throw new Error("useCloud must be used within a CloudProvider");
	}
	return context;
}

export function CloudDeck({
	workspace,
	activeSession,
	skipInitialScrollback,
}: {
	workspace: string;
	activeSession: CloudSession | null;
	skipInitialScrollback: boolean;
}) {
	const outletRef = useRef<HTMLDivElement>(null);
	const {
		ensureSession,
		getHost,
		registerWorkspaceOutlet,
		setActiveSession,
	} = useCloud();
	const activeSessionKey = activeSession ? cloudSessionKey(activeSession) : null;
	const activeHost = getHost(activeSessionKey);

	useEffect(() => {
		registerWorkspaceOutlet(workspace, outletRef.current);
		return () => {
			registerWorkspaceOutlet(workspace, null);
		};
	}, [registerWorkspaceOutlet, workspace]);

	useEffect(() => {
		if (activeSession) {
			ensureSession(activeSession, {
				skipInitialScrollback,
			});
		}
		setActiveSession(workspace, activeSessionKey);
		return () => {
			setActiveSession(null, null);
		};
	}, [
		activeSession?.attachmentId,
		activeSession?.kind,
		activeSession?.name,
		activeSession?.unread,
		activeSession?.working,
		activeSession?.workspace,
		activeSessionKey,
		ensureSession,
		setActiveSession,
		skipInitialScrollback,
		workspace,
	]);

	return (
		<div className="flex-1 min-h-0 bg-surface relative">
			{activeSession && (!activeHost || activeHost.status !== "ready") && (
				<div className="absolute inset-0 flex items-center justify-center z-10">
					<div className="flex items-center gap-2 text-[11px] text-text-muted">
						{activeHost?.status === "error" ? (
							<span>{activeHost.errorMessage ?? "Session failed to attach"}</span>
						) : (
							<>
								<Loader />
								<span>Connecting to session...</span>
							</>
						)}
					</div>
				</div>
			)}
			<div ref={outletRef} className="h-full w-full" />
		</div>
	);
}
