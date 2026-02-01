import { createFileRoute } from "@tanstack/react-router";
import { useRuns } from "@/hooks/use-runs";
import type { Run, RunStatus } from "@/lib/types";

export const Route = createFileRoute("/")({
  component: Index,
});

function Index() {
  const { data: runs, isLoading, error } = useRuns();

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-12">
        <div className="text-muted-foreground">Loading runs...</div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="rounded-md border border-destructive/50 bg-destructive/10 p-4">
        <p className="text-destructive">Failed to load runs: {error.message}</p>
        <button
          onClick={() => window.location.reload()}
          className="mt-2 text-sm underline"
        >
          Retry
        </button>
      </div>
    );
  }

  if (!runs || runs.length === 0) {
    return (
      <div className="py-12 text-center">
        <h1 className="text-2xl font-bold">Runs</h1>
        <p className="mt-2 text-muted-foreground">No runs found</p>
      </div>
    );
  }

  return (
    <div>
      <h1 className="mb-6 text-2xl font-bold">Runs</h1>
      <div className="grid gap-4">
        {runs.map((run) => (
          <RunCard key={run.id} run={run} />
        ))}
      </div>
    </div>
  );
}

const statusStyles: Record<RunStatus, string> = {
  Pending: "bg-muted text-muted-foreground",
  Running: "bg-blue-500/20 text-blue-600 dark:text-blue-400",
  Completed: "bg-green-500/20 text-green-600 dark:text-green-400",
  Failed: "bg-red-500/20 text-red-600 dark:text-red-400",
  Canceled: "bg-muted text-muted-foreground",
  Paused: "bg-yellow-500/20 text-yellow-600 dark:text-yellow-400",
};

function RunCard({ run }: { run: Run }) {
  const workspaceName = run.workspace_root.split("/").pop() ?? run.workspace_root;

  // TODO: Convert to Link once /runs/$runId route exists (Phase 4)
  return (
    <div className="rounded-lg border border-border bg-card p-4">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0 flex-1">
          <h2 className="truncate font-medium">{run.name}</h2>
          <p className="mt-1 truncate text-sm text-muted-foreground">
            {workspaceName}
          </p>
        </div>
        <span
          className={`shrink-0 rounded-full px-2 py-1 text-xs font-medium ${statusStyles[run.status]}`}
        >
          {run.status}
        </span>
      </div>
      <div className="mt-3 flex items-center gap-4 text-xs text-muted-foreground">
        <span>Created: {formatTime(run.created_at)}</span>
        <span>Updated: {formatTime(run.updated_at)}</span>
      </div>
    </div>
  );
}

function formatTime(iso: string): string {
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}
