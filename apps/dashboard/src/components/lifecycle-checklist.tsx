import type { Run, RunEvent, LifecycleStep } from "@/lib/types";

interface LifecycleChecklistProps {
  run: Run;
  events: RunEvent[];
}

/**
 * Lifecycle checklist derived from run events.
 * See spec §5: Derive checklist from events for completed runs.
 *
 * Steps derived:
 * - Run started
 * - Implementation completed (N iterations)
 * - Review passed
 * - Verification passed
 * - Worktree merged to branch
 * - Worktree cleaned up
 * - Branch ready for review
 */
export function LifecycleChecklist({ run, events }: LifecycleChecklistProps) {
  const steps = deriveLifecycleSteps(run, events);

  if (steps.length === 0) {
    return null;
  }

  return (
    <div className="rounded-lg border border-border bg-card">
      <div className="border-b border-border px-4 py-2">
        <h2 className="font-medium">Lifecycle</h2>
      </div>
      <ul className="divide-y divide-border">
        {steps.map((step, index) => (
          <li key={index} className="flex items-start gap-3 px-4 py-3">
            <div className="mt-0.5">
              {step.completed ? (
                <CheckIcon className="h-4 w-4 text-green-500" />
              ) : (
                <CircleIcon className="h-4 w-4 text-muted-foreground" />
              )}
            </div>
            <div className="flex-1 min-w-0">
              <p
                className={
                  step.completed
                    ? "text-sm text-foreground"
                    : "text-sm text-muted-foreground"
                }
              >
                {step.label}
              </p>
              {step.timestamp && (
                <p className="text-xs text-muted-foreground mt-0.5">
                  {formatTimestamp(step.timestamp)}
                </p>
              )}
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}

function deriveLifecycleSteps(run: Run, events: RunEvent[]): LifecycleStep[] {
  const steps: LifecycleStep[] = [];

  // Run started
  const startEvent = events.find((e) => e.event_type === "run_started");
  steps.push({
    label: "Run started",
    completed: !!startEvent,
    timestamp: startEvent ? new Date(startEvent.timestamp).toISOString() : undefined,
  });

  // Count implementation iterations
  const implCompletedEvents = events.filter(
    (e) =>
      e.event_type === "step_completed" &&
      e.payload?.phase === "Implementation" &&
      e.payload?.status === "Succeeded"
  );
  const implIterations = implCompletedEvents.length;
  const lastImplEvent = implCompletedEvents[implCompletedEvents.length - 1];
  steps.push({
    label:
      implIterations > 0
        ? `Implementation completed (${implIterations} iteration${implIterations > 1 ? "s" : ""})`
        : "Implementation",
    completed: implIterations > 0,
    timestamp: lastImplEvent ? new Date(lastImplEvent.timestamp).toISOString() : undefined,
  });

  // Review passed
  const reviewEvent = events.find(
    (e) =>
      e.event_type === "step_completed" &&
      e.payload?.phase === "Review" &&
      e.payload?.status === "Succeeded"
  );
  steps.push({
    label: "Review passed",
    completed: !!reviewEvent,
    timestamp: reviewEvent ? new Date(reviewEvent.timestamp).toISOString() : undefined,
  });

  // Verification passed
  const verifyEvent = events.find(
    (e) =>
      e.event_type === "step_completed" &&
      e.payload?.phase === "Verification" &&
      e.payload?.status === "Succeeded"
  );
  steps.push({
    label: "Verification passed",
    completed: !!verifyEvent,
    timestamp: verifyEvent ? new Date(verifyEvent.timestamp).toISOString() : undefined,
  });

  // Worktree merged to branch
  const mergeEvent = events.find(
    (e) =>
      e.event_type === "step_completed" &&
      e.payload?.phase === "Merge" &&
      e.payload?.status === "Succeeded"
  );
  steps.push({
    label: "Worktree merged to branch",
    completed: !!mergeEvent,
    timestamp: mergeEvent ? new Date(mergeEvent.timestamp).toISOString() : undefined,
  });

  // Worktree cleaned up (based on run completion with worktree provider)
  const runCompleted = events.find((e) => e.event_type === "run_completed");
  const wasCleanedUp = runCompleted && run.status === "Completed";
  steps.push({
    label: "Worktree cleaned up",
    completed: !!wasCleanedUp,
    timestamp: runCompleted ? new Date(runCompleted.timestamp).toISOString() : undefined,
  });

  // Branch ready for review (final state for completed runs with branch)
  const branchReady = run.status === "Completed" && run.worktree?.run_branch;
  steps.push({
    label: "Branch ready for review",
    completed: !!branchReady,
  });

  return steps;
}

function formatTimestamp(isoString: string): string {
  const date = new Date(isoString);
  return date.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function CheckIcon({ className }: { className?: string }) {
  return (
    <svg
      className={className}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M20 6L9 17l-5-5" />
    </svg>
  );
}

function CircleIcon({ className }: { className?: string }) {
  return (
    <svg
      className={className}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <circle cx="12" cy="12" r="10" />
    </svg>
  );
}
