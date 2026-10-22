import "@/app/globals.css";
import type { Metadata, Viewport } from "next";
import { Toaster } from "@/components/ui/toaster";
import { ThemeProvider } from "@/components/theme-provider";
import { ProjectProvider } from "@/components/project-context";
import { SiteNav } from "@/components/site-nav";
import { ConfigBanner } from "@/components/config-banner";

export const metadata: Metadata = {
  title: "Dashboard",
  description: "Code intelligence dashboard for coding agents",
};

export const viewport: Viewport = {
  width: "device-width",
  initialScale: 1,
  themeColor: [
    { media: "(prefers-color-scheme: light)", color: "#ffffff" },
    { media: "(prefers-color-scheme: dark)", color: "#0a0f1a" },
  ],
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" suppressHydrationWarning>
      <body className="min-h-dvh bg-background text-foreground antialiased selection:bg-primary/20 selection:text-primary">
        <ThemeProvider>
          <ProjectProvider>
            <SiteNav />
            <ConfigBanner />
            <main className="mx-auto w-full max-w-[1500px] px-4 pb-16 pt-[4.25rem] sm:px-6 lg:px-8">
              {children}
            </main>
            <Toaster />
          </ProjectProvider>
        </ThemeProvider>
      </body>
    </html>
  );
}
