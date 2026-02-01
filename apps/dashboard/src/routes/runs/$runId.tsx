import { createFileRoute, Link } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef } from "react";
import { useRun } from "@/hooks/use-run";
import { useSteps } from "@/hooks/use-steps";
import { useRunEvents } from "@/hooks/use-run-events";
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

  // Track event count to detect new events
  const lastEventCountRef = useRef(0);

  // Invalidate run/steps queries when new events arrive
  useEffect(() => {
    if (events.length > lastEventCountRef.current) {
      const newEvents = events.slice(lastEventCountRef.current);
      lastEventCountRef.current = events.length;

      // Check if any event should trigger a refresh
      const shouldRefreshRun = newEvents.some((e) =>
        ["run_started", "run_completed", "run_failed", "run_canceled", "run_paused"].includes(
          e.event_type
        )
      );
      const shouldRefreshSteps = newEvents.some((e) =>
        ["step_started", "step_completed", "step_failed"].includes(e.event_type)
      );

      if (shouldRefreshRun) {
        queryClient.invalidateQueries({ queryKey: ["run", runId] });
      }
      if (shouldRefreshSteps) {
        queryClient.invalidateQueries({ queryKey: ["steps", runId] });
      }
    }
  }, [events, runId, queryClient]);

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
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <Link to="/" className="text-sm text-muted-foreground hover:underline">
          &larr; Back to runs
        </Link>
        <div className="flex items-center gap-2 text-sm">
          <span
            className={`h-2 w-2 rounded-full ${
              eventsConnected ? "bg-green-500" : "bg-yellow-500 animate-pulse"
            }`}
          />
          <span className="text-muted-foreground">
            {eventsConnected ? "Live" : "Reconnecting..."}
          </span>
        </div>
      </div>

      <RunDetailComponent run={run} />

      {stepsLoading ? (
        <div className="rounded-lg border border-border bg-card p-4">
          <div className="text-sm text-muted-foreground">Loading steps...</div>
        </div>
      ) : steps && steps.length > 0 ? (
        <StepTimeline steps={steps} />
      ) : null}

      <LifecycleChecklist run={run} events={events} />

      <LogViewer runId={runId} />
    </div>
  );
}
