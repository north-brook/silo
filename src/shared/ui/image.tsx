import type { ImgHTMLAttributes } from "react";

type ImageProps = Omit<ImgHTMLAttributes<HTMLImageElement>, "src"> & {
	alt: string;
	height?: number | string;
	src: string;
	unoptimized?: boolean;
	width?: number | string;
};

export default function Image({
	alt,
	unoptimized: _unoptimized,
	...props
}: ImageProps) {
	return <img alt={alt} {...props} />;
}
