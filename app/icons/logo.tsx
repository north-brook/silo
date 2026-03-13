export function LogoIcon({
	className,
	height,
}: {
	className?: string;
	height: number;
}) {
	return (
		<svg
			width={height}
			height={height}
			viewBox="0 0 600 600"
			xmlns="http://www.w3.org/2000/svg"
			className={className}
		>
			<title>Silo</title>
			<path
				fill="#FFFFFF"
				d="M600 600H0V450H450V300H600V600ZM600 150H150V300H0V0H600V150Z"
			/>
		</svg>
	);
}
