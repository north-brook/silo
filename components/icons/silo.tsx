const VIEWBOX_WIDTH = 2250;
const VIEWBOX_HEIGHT = 600;

export function SiloIcon({
	className,
	height,
}: {
	className?: string;
	height: number;
}) {
	const width = height * (VIEWBOX_WIDTH / VIEWBOX_HEIGHT);
	return (
		<svg
			width={width}
			height={height}
			viewBox="0 0 2250 600"
			xmlns="http://www.w3.org/2000/svg"
			className={className}
		>
			<title>Silo</title>
			<path
				fill="#FFFFFF"
				d="M600 600H0V450H450V300H600V600ZM900 600H750V0H900V600ZM1500 600H1200V450H1500V600ZM1800 450H2100V600H1650V150H1800V450ZM1200 450H1050V0H1200V450ZM2250 450H2100V150H1800V0H2250V450ZM600 150H150V300H0V0H600V150Z"
			/>
		</svg>
	);
}
