import "@/app/globals.css";
import type { Metadata } from "next";
import { Toaster } from "@/components/ui/toaster";
import Link from "next/link";

export const metadata: Metadata = {
  title: "imc Dashboard",
  description: "IndexMyCode dashboard - code intelligence for coding agents",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body className="bg-background text-foreground">
        <nav className="fixed top-0 left-0 right-0 h-14 bg-background border-b border-border z-50 flex items-center px-6">
          <Link href="/" className="flex items-center gap-2 font-bold text-lg text-primary">
            <span className="text-sm text-muted-foreground">imc</span>
            <span>Dashboard</span>
          </Link>
          <div className="ml-auto flex items-center gap-4">
            <Link href="/dashboard" className="text-sm font-medium hover:text-primary">
              Projects
            </Link>
            <Link href="/search" className="text-sm font-medium hover:text-primary">
              Search
            </Link>
            <Link href="/trace" className="text-sm font-medium hover:text-primary">
              Trace
            </Link>
            <Link href="/memory" className="text-sm font-medium hover:text-primary">
              Memory
            </Link>
            <Link href="/compression" className="text-sm font-medium hover:text-primary">
              Compression
            </Link>
          </div>
        </nav>
        <main className="pt-20 pb-8 px-6">{children}</main>
        <Toaster />
      </body>
    </html>
  );
}
