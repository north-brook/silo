const MAC_DOWNLOAD_URL =
	"https://github.com/north-brook/silo/releases/latest/download/Silo-macos-arm64-dmg.dmg";

export default function Home() {
	return (
		<main className="min-h-screen overflow-hidden bg-canvas text-ink">
			<div className="pointer-events-none absolute inset-0 bg-[radial-gradient(circle_at_top,_rgba(255,186,94,0.32),_transparent_38%),linear-gradient(160deg,_rgba(255,255,255,0.9),_rgba(255,247,235,0.78)_52%,_rgba(255,233,205,0.55))]" />
			<div className="pointer-events-none absolute inset-x-0 top-0 h-72 bg-[linear-gradient(180deg,_rgba(20,18,15,0.1),_transparent)]" />

			<section className="relative mx-auto flex min-h-screen max-w-6xl flex-col justify-center gap-12 px-6 py-16 md:px-10 lg:px-12">
				<div className="max-w-2xl space-y-6">
					<p className="inline-flex items-center rounded-full border border-ink/10 bg-white/70 px-3 py-1 font-mono text-[11px] uppercase tracking-[0.28em] text-ink/70 shadow-sm backdrop-blur">
						Cloud workspaces, local control
					</p>
					<div className="space-y-4">
						<h1 className="max-w-xl text-5xl font-semibold tracking-[-0.06em] text-ink sm:text-6xl">
							Silo keeps your remote dev environment feeling local.
						</h1>
						<p className="max-w-xl text-lg leading-8 text-ink/68 sm:text-xl">
							Launch cloud-hosted workspaces, open terminals and browser tabs, and
							manage the full session from one desktop app.
						</p>
					</div>

					<div className="flex flex-col items-start gap-3 sm:flex-row sm:items-center">
						<a
							className="inline-flex items-center justify-center rounded-full bg-ink px-6 py-3 text-sm font-medium text-white shadow-[0_16px_40px_rgba(17,14,11,0.22)] transition hover:-translate-y-0.5 hover:bg-ink/92"
							href={MAC_DOWNLOAD_URL}
						>
							Download for Mac
						</a>
						<p className="font-mono text-xs uppercase tracking-[0.22em] text-ink/48">
							Apple Silicon build
						</p>
					</div>
				</div>

				<div className="relative">
					<div className="absolute inset-x-10 top-6 h-full rounded-[2rem] bg-amber-300/20 blur-3xl" />
					<div className="relative overflow-hidden rounded-[2rem] border border-ink/10 bg-white/75 p-4 shadow-[0_30px_120px_rgba(44,28,12,0.18)] backdrop-blur xl:p-5">
						<div className="rounded-[1.4rem] border border-ink/8 bg-[linear-gradient(180deg,_rgba(255,255,255,0.98),_rgba(250,243,234,0.96))] p-4">
							<div className="flex items-center justify-between rounded-2xl border border-ink/8 bg-white/90 px-4 py-3">
								<div>
									<p className="font-mono text-[11px] uppercase tracking-[0.24em] text-ink/45">
										Placeholder product shot
									</p>
									<p className="mt-1 text-sm font-medium text-ink/75">
										Project switcher, live terminals, and cloud workspace controls
									</p>
								</div>
								<div className="flex gap-2">
									<span className="h-3 w-3 rounded-full bg-[#ff6d5f]" />
									<span className="h-3 w-3 rounded-full bg-[#fdbc43]" />
									<span className="h-3 w-3 rounded-full bg-[#34c84a]" />
								</div>
							</div>

							<div className="mt-4 grid gap-4 lg:grid-cols-[1.1fr_1.7fr]">
								<div className="space-y-3 rounded-[1.4rem] border border-ink/8 bg-[#171716] p-4 text-white">
									<div className="flex items-center justify-between">
										<p className="font-mono text-[11px] uppercase tracking-[0.24em] text-white/45">
											Terminal
										</p>
										<span className="rounded-full bg-emerald-400/18 px-2 py-1 font-mono text-[10px] uppercase tracking-[0.18em] text-emerald-300">
											Connected
										</span>
									</div>
									<div className="space-y-2 font-mono text-sm text-white/78">
										<p>$ gcloud compute ssh silo-dev-01</p>
										<p className="text-white/52">Attaching remote workspace...</p>
										<p>$ bun run test</p>
										<p className="text-amber-200">watching logs, files, and browser tabs</p>
									</div>
								</div>

								<div className="space-y-4 rounded-[1.4rem] border border-ink/8 bg-[#fcfbf7] p-4">
									<div className="flex items-center justify-between">
										<div>
											<p className="font-mono text-[11px] uppercase tracking-[0.24em] text-ink/45">
												Workspace overview
											</p>
											<p className="mt-1 text-sm text-ink/68">
												One local shell for project state, browser sessions, and lifecycle.
											</p>
										</div>
										<div className="rounded-full border border-ink/10 bg-white px-3 py-1 font-mono text-[11px] uppercase tracking-[0.18em] text-ink/48">
										+2 tabs
										</div>
									</div>

									<div className="grid gap-3 md:grid-cols-3">
										{[
											["Workspace", "Ready"],
											["Browser", "Attached"],
											["Sync", "Live"],
										].map(([label, value]) => (
											<div
												key={label}
												className="rounded-2xl border border-ink/8 bg-white px-4 py-3 shadow-sm"
											>
												<p className="font-mono text-[11px] uppercase tracking-[0.2em] text-ink/40">
													{label}
												</p>
												<p className="mt-3 text-xl font-semibold tracking-[-0.04em] text-ink">
													{value}
												</p>
											</div>
										))}
									</div>

									<div className="rounded-[1.4rem] border border-dashed border-ink/12 bg-[linear-gradient(135deg,_rgba(255,194,102,0.18),_rgba(255,255,255,0.9)_55%,_rgba(255,222,173,0.28))] p-6">
										<p className="max-w-lg text-lg leading-8 text-ink/70">
											This placeholder panel will be replaced with a real product screenshot once the production app visuals are ready.
										</p>
									</div>
								</div>
							</div>
						</div>
					</div>
				</div>
			</section>
		</main>
	);
}
