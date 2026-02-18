import { createFileRoute, Link, Outlet, useRouterState } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef } from "react";
import { useRun } from "@/hooks/use-run";
import { useSteps } from "@/hooks/use-steps";
import { useRunEvents } from "@/hooks/use-run-events";
import { useEscapeToGoBack } from "@/hooks/use-keyboard-navigation";
import { RunDetail as RunDetailComponent } from "@/components/run-detail";
import { StepTimeline } from "@/components/step-timeline";
import { LogViewer } from "@/components/log-viewer";
import { LifecycleChecklist } from "@/components/lifecycle-checklist";

export const Route = createFileRoute("/runs/$runId")({
  component: RunDetailPage,
});

function RunDetailPage() {
  const { runId } = Route.useParams();
  const queryClient = useQueryClient();
  const { run, isLoading, error } = useRun(runId);
  const { data: steps, isLoading: stepsLoading } = useSteps(runId);
  const { events, connected: eventsConnected } = useRunEvents(runId);
  const isReviewRoute = useRouterState({
    select: (state) => state.matches.some((match) => match.routeId === "/runs/$runId/review"),
  });
  const statusIndicator = (() => {
    if (!run) {
      return {
        dotClass: eventsConnected ? "bg-green-500" : "bg-yellow-500 animate-pulse",
        label: eventsConnected ? "Live" : "Reconnecting...",
      };
    }

    if (run.status === "Paused") {
      return { dotClass: "bg-yellow-500", label: "Paused" };
    }

    if (["Completed", "Failed", "Canceled"].includes(run.status)) {
      const dotClass =
        run.status === "Failed"
          ? "bg-red-500"
          : run.status === "Canceled"
          ? "bg-yellow-500"
          : "bg-green-500";
      return { dotClass, label: run.status };
    }

    return {
      dotClass: eventsConnected ? "bg-green-500" : "bg-yellow-500 animate-pulse",
      label: eventsConnected ? "Live" : "Reconnecting...",
    };
  })();

  // Enable Escape key to go back to run list
  useEscapeToGoBack();

  // Track event count to detect new events
  const lastEventCountRef = useRef(0);

  // Invalidate run/steps queries when new events arrive
  useEffect(() => {
    if (events.length > lastEventCountRef.current) {
      const newEvents = events.slice(lastEventCountRef.current);
      lastEventCountRef.current = events.length;

      // Check if any event should trigger a refresh
      const shouldRefreshRun = newEvents.some((e) => {
        const eventType = e.event_type.toLowerCase();
        return ["run_started", "run_completed", "run_failed", "run_canceled", "run_paused"].includes(
          eventType
        );
      });
      const shouldRefreshSteps = newEvents.some((e) => {
        const eventType = e.event_type.toLowerCase();
        return ["step_started", "step_finished", "step_failed"].includes(eventType);
      });

      if (shouldRefreshRun) {
        queryClient.invalidateQueries({ queryKey: ["run", runId] });
      }
      if (shouldRefreshSteps) {
        queryClient.invalidateQueries({ queryKey: ["steps", runId] });
        queryClient.invalidateQueries({ queryKey: ["run-diff", runId] });
      }
    }
  }, [events, runId, queryClient]);

  if (isReviewRoute) {
    return <Outlet />;
  }

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

  return (
    <div className="space-y-4 sm:space-y-6">
      <div className="flex items-center justify-between">
        <Link to="/" className="text-xs text-muted-foreground hover:underline sm:text-sm">
          &larr; Back to runs
        </Link>
        <div className="flex items-center gap-3">
          {run.worktree?.run_branch && (run.status === "Completed" || run.status === "Paused" || run.status === "Running") && (
            <Link
              to="/runs/$runId/review"
              params={{ runId }}
              className="px-3 py-1.5 text-sm rounded bg-primary text-primary-foreground hover:bg-primary/90 transition-colors"
            >
              {run.status === "Running" ? "View Changes" : "Review Changes"}
            </Link>
          )}
          <div className="flex items-center gap-2 text-xs sm:text-sm">
            <span
              className={`h-2 w-2 rounded-full ${statusIndicator.dotClass}`}
            />
            <span className="text-muted-foreground">
              {statusIndicator.label}
            </span>
          </div>
        </div>
      </div>

      <RunDetailComponent run={run} />

      {stepsLoading ? (
        <div className="rounded-lg border border-border bg-card p-4">
          <div className="text-sm text-muted-foreground">Loading steps...</div>
        </div>
      ) : steps && steps.length > 0 ? (
        <StepTimeline steps={steps} runStatus={run.status} />
      ) : (
        <div className="rounded-lg border border-border bg-card p-4">
          <div className="text-sm text-muted-foreground">No steps yet</div>
        </div>
      )}

      <LifecycleChecklist run={run} events={events} steps={steps ?? []} />

      <LogViewer runId={runId} runStatus={run.status} />
    </div>
  );
}
