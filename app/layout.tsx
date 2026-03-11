import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "silo",
  description: "Tauri app powered by Next.js",
};

type RootLayoutProps = Readonly<{
  children: React.ReactNode;
}>;

export default function RootLayout({ children }: RootLayoutProps) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
