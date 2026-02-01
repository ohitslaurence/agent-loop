import { createRootRoute, Link, Outlet } from "@tanstack/react-router";
import { Toaster } from "sonner";
import { DaemonStatusBanner } from "@/components/daemon-status-banner";

export const Route = createRootRoute({
  component: RootLayout,
});

function RootLayout() {
  return (
    <div className="min-h-screen bg-background text-foreground">
      <DaemonStatusBanner />
      <header className="border-b border-border">
        <div className="container mx-auto flex items-center justify-between p-4">
          <Link to="/" className="text-xl font-semibold">
            Dashboard
          </Link>
          {/* Workspace switcher will be added here */}
        </div>
      </header>
      <main className="container mx-auto p-4">
        <Outlet />
      </main>
      <Toaster
        position="bottom-right"
        toastOptions={{
          className: "bg-background text-foreground border-border",
        }}
      />
    </div>
  );
}
