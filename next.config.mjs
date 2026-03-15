/** @type {import('next').NextConfig} */
const nextConfig = {
	output: "export",
	trailingSlash: true,
	reactStrictMode: process.env.NODE_ENV === "production",
	images: {
		unoptimized: true,
	},
	devIndicators: false,
};

export default nextConfig;
