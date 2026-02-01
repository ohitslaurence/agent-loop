import { Link } from "@tanstack/react-router";
import type { Run, RunStatus } from "@/lib/types";

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

interface RunCardProps {
  run: Run;
  isSelected?: boolean;
}

export function RunCard({ run, isSelected = false }: RunCardProps) {
  const workspaceName = run.workspace_root.split("/").pop() ?? run.workspace_root;

  return (
    <Link
      to="/runs/$runId"
      params={{ runId: run.id }}
      data-run-card
      className={`block rounded-lg border bg-card p-4 transition-colors ${
        isSelected
          ? "border-primary ring-2 ring-primary/20"
          : "border-border hover:border-foreground/20"
      }`}
    >
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
    </Link>
  );
}
