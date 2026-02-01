import { Link } from "@tanstack/react-router";
import type { Run, RunStatus, ReviewStatus } from "@/lib/types";

const statusStyles: Record<RunStatus, string> = {
  Pending: "bg-muted text-muted-foreground",
  Running: "bg-blue-500/20 text-blue-600 dark:text-blue-400",
  Completed: "bg-green-500/20 text-green-600 dark:text-green-400",
  Failed: "bg-red-500/20 text-red-600 dark:text-red-400",
  Canceled: "bg-muted text-muted-foreground",
  Paused: "bg-yellow-500/20 text-yellow-600 dark:text-yellow-400",
};

const reviewStatusStyles: Record<ReviewStatus, { style: string; label: string }> = {
  Pending: { style: "", label: "" },
  Reviewed: { style: "bg-blue-500/20 text-blue-600 dark:text-blue-400", label: "Reviewed" },
  Scrapped: { style: "bg-muted text-muted-foreground", label: "Scrapped" },
  Merged: { style: "bg-purple-500/20 text-purple-600 dark:text-purple-400", label: "Merged" },
  PrCreated: { style: "bg-indigo-500/20 text-indigo-600 dark:text-indigo-400", label: "PR Created" },
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
        <div className="flex shrink-0 items-center gap-2">
          {run.review_status && run.review_status !== "Pending" && (
            <span
              className={`rounded-full px-2 py-1 text-xs font-medium ${reviewStatusStyles[run.review_status].style}`}
            >
              {reviewStatusStyles[run.review_status].label}
            </span>
          )}
          <span
            className={`rounded-full px-2 py-1 text-xs font-medium ${statusStyles[run.status]}`}
          >
            {run.status}
          </span>
        </div>
      </div>
      <div className="mt-3 flex flex-col gap-1 text-xs text-muted-foreground sm:flex-row sm:items-center sm:gap-4">
        <span>Created: {formatTime(run.created_at)}</span>
        <span>Updated: {formatTime(run.updated_at)}</span>
        {run.pr_url && (
          <a
            href={run.pr_url}
            target="_blank"
            rel="noopener noreferrer"
            onClick={(e) => e.stopPropagation()}
            className="text-primary hover:underline"
          >
            View PR
          </a>
        )}
      </div>
    </Link>
  );
}
