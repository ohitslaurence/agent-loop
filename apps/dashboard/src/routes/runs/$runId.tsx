import { createFileRoute, Link } from "@tanstack/react-router";
import { useRun } from "@/hooks/use-run";
import type { RunStatus } from "@/lib/types";

export const Route = createFileRoute("/runs/$runId")({
  component: RunDetail,
});

const statusStyles: Record<RunStatus, string> = {
  Pending: "bg-muted text-muted-foreground",
  Running: "bg-blue-500/20 text-blue-600 dark:text-blue-400",
  Completed: "bg-green-500/20 text-green-600 dark:text-green-400",
  Failed: "bg-red-500/20 text-red-600 dark:text-red-400",
  Canceled: "bg-muted text-muted-foreground",
  Paused: "bg-yellow-500/20 text-yellow-600 dark:text-yellow-400",
};

function formatTime(iso: string): string {
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

function RunDetail() {
  const { runId } = Route.useParams();
  const { run, isLoading, error } = useRun(runId);

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-12">
        <div className="text-muted-foreground">Loading run...</div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="space-y-4">
        <Link to="/" className="text-sm text-muted-foreground hover:underline">
          &larr; Back to runs
        </Link>
        <div className="rounded-md border border-destructive/50 bg-destructive/10 p-4">
          <p className="text-destructive">Failed to load run: {error.message}</p>
        </div>
      </div>
    );
  }

  if (!run) {
    return (
      <div className="space-y-4">
        <Link to="/" className="text-sm text-muted-foreground hover:underline">
          &larr; Back to runs
        </Link>
        <div className="py-12 text-center">
          <p className="text-muted-foreground">Run not found</p>
        </div>
      </div>
    );
  }

  const workspaceName = run.workspace_root.split("/").pop() ?? run.workspace_root;

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-4">
        <Link to="/" className="text-sm text-muted-foreground hover:underline">
          &larr; Back to runs
        </Link>
      </div>

      <div className="space-y-4">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h1 className="text-2xl font-bold">{run.name}</h1>
            <p className="mt-1 text-muted-foreground">{workspaceName}</p>
          </div>
          <span
            className={`shrink-0 rounded-full px-3 py-1 text-sm font-medium ${statusStyles[run.status]}`}
          >
            {run.status}
          </span>
        </div>

        <div className="rounded-lg border border-border bg-card p-4">
          <h2 className="mb-3 font-medium">Details</h2>
          <dl className="grid gap-2 text-sm">
            <div className="flex gap-2">
              <dt className="text-muted-foreground">ID:</dt>
              <dd className="font-mono">{run.id}</dd>
            </div>
            <div className="flex gap-2">
              <dt className="text-muted-foreground">Workspace:</dt>
              <dd className="font-mono">{run.workspace_root}</dd>
            </div>
            <div className="flex gap-2">
              <dt className="text-muted-foreground">Spec:</dt>
              <dd className="font-mono">{run.spec_path}</dd>
            </div>
            {run.plan_path && (
              <div className="flex gap-2">
                <dt className="text-muted-foreground">Plan:</dt>
                <dd className="font-mono">{run.plan_path}</dd>
              </div>
            )}
            <div className="flex gap-2">
              <dt className="text-muted-foreground">Created:</dt>
              <dd>{formatTime(run.created_at)}</dd>
            </div>
            <div className="flex gap-2">
              <dt className="text-muted-foreground">Updated:</dt>
              <dd>{formatTime(run.updated_at)}</dd>
            </div>
          </dl>
        </div>

        {run.worktree && (
          <div className="rounded-lg border border-border bg-card p-4">
            <h2 className="mb-3 font-medium">Worktree</h2>
            <dl className="grid gap-2 text-sm">
              <div className="flex gap-2">
                <dt className="text-muted-foreground">Path:</dt>
                <dd className="font-mono">{run.worktree.worktree_path}</dd>
              </div>
              <div className="flex gap-2">
                <dt className="text-muted-foreground">Branch:</dt>
                <dd className="font-mono">{run.worktree.run_branch}</dd>
              </div>
              <div className="flex gap-2">
                <dt className="text-muted-foreground">Base:</dt>
                <dd className="font-mono">{run.worktree.base_branch}</dd>
              </div>
              {run.worktree.merge_target_branch && (
                <div className="flex gap-2">
                  <dt className="text-muted-foreground">Merge Target:</dt>
                  <dd className="font-mono">{run.worktree.merge_target_branch}</dd>
                </div>
              )}
              <div className="flex gap-2">
                <dt className="text-muted-foreground">Strategy:</dt>
                <dd>{run.worktree.merge_strategy}</dd>
              </div>
              <div className="flex gap-2">
                <dt className="text-muted-foreground">Provider:</dt>
                <dd>{run.worktree.provider}</dd>
              </div>
            </dl>
          </div>
        )}

        {/* Step timeline will be added in Phase 4 when step-timeline.tsx is created */}
      </div>
    </div>
  );
}
