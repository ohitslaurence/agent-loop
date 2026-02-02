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
              ) : step.inProgress ? (
                <SpinnerIcon className="h-4 w-4 text-blue-500 animate-spin" />
              ) : (
                <CircleIcon className="h-4 w-4 text-muted-foreground" />
              )}
            </div>
            <div className="flex-1 min-w-0">
              <p
                className={
                  step.completed
                    ? "text-sm text-foreground"
                    : step.inProgress
                    ? "text-sm text-blue-600 dark:text-blue-400 font-medium"
                    : "text-sm text-muted-foreground"
                }
              >
                {step.label}
                {step.inProgress && <span className="ml-2 text-xs">Running...</span>}
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

  // Helper to match event types case-insensitively
  const matchType = (e: RunEvent, type: string) =>
    e.event_type.toLowerCase() === type.toLowerCase();

  // Helper to safely get phase from payload
  const getPhase = (e: RunEvent): string | undefined => {
    const phase = e.payload?.phase;
    return typeof phase === "string" ? phase.toLowerCase() : undefined;
  };

  // Run started (look for first step start as proxy for run start)
  const startEvent = events.find((e) => matchType(e, "STEP_STARTED"));
  steps.push({
    label: "Run started",
    completed: !!startEvent,
    inProgress: false,
    timestamp: startEvent ? new Date(startEvent.timestamp).toISOString() : undefined,
  });

  // Count implementation iterations (STEP_FINISHED with phase=implementation)
  const implStartedEvents = events.filter(
    (e) =>
      matchType(e, "STEP_STARTED") &&
      getPhase(e) === "implementation"
  );
  const implCompletedEvents = events.filter(
    (e) =>
      matchType(e, "STEP_FINISHED") &&
      getPhase(e) === "implementation"
  );
  const implIterations = implCompletedEvents.length;
  const implInProgress = implStartedEvents.length > implCompletedEvents.length;
  const lastImplEvent = implCompletedEvents[implCompletedEvents.length - 1];
  steps.push({
    label:
      implIterations > 0
        ? `Implementation (${implIterations} iteration${implIterations > 1 ? "s" : ""})`
        : "Implementation",
    completed: implIterations > 0 && !implInProgress,
    inProgress: implInProgress,
    timestamp: lastImplEvent ? new Date(lastImplEvent.timestamp).toISOString() : undefined,
  });

  // Self-review completed
  const reviewStarted = events.find(
    (e) =>
      matchType(e, "STEP_STARTED") &&
      getPhase(e) === "review"
  );
  const reviewEvent = events.find(
    (e) =>
      matchType(e, "STEP_FINISHED") &&
      getPhase(e) === "review"
  );
  steps.push({
    label: "Self-review (automated)",
    completed: !!reviewEvent,
    inProgress: !!reviewStarted && !reviewEvent,
    timestamp: reviewEvent ? new Date(reviewEvent.timestamp).toISOString() : undefined,
  });

  // Verification completed
  const verifyStarted = events.find(
    (e) =>
      matchType(e, "STEP_STARTED") &&
      getPhase(e) === "verification"
  );
  const verifyEvent = events.find(
    (e) =>
      matchType(e, "STEP_FINISHED") &&
      getPhase(e) === "verification"
  );
  steps.push({
    label: "Verification",
    completed: !!verifyEvent,
    inProgress: !!verifyStarted && !verifyEvent,
    timestamp: verifyEvent ? new Date(verifyEvent.timestamp).toISOString() : undefined,
  });

  // Worktree merged/removed
  const worktreeRemoved = events.find((e) => matchType(e, "WORKTREE_REMOVED"));
  steps.push({
    label: "Worktree cleaned up",
    completed: !!worktreeRemoved,
    inProgress: false,
    timestamp: worktreeRemoved ? new Date(worktreeRemoved.timestamp).toISOString() : undefined,
  });

  // Run completed
  const runCompleted = events.find((e) => matchType(e, "RUN_COMPLETED"));
  steps.push({
    label: "Run completed",
    completed: !!runCompleted,
    inProgress: run.status === "Running",
    timestamp: runCompleted ? new Date(runCompleted.timestamp).toISOString() : undefined,
  });

  // Branch ready for review (final state for completed runs with branch)
  const branchReady = run.status === "Completed" && run.worktree?.run_branch;
  steps.push({
    label: "Branch ready for review",
    completed: !!branchReady,
    inProgress: false,
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

function SpinnerIcon({ className }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none">
      <circle
        className="opacity-25"
        cx="12"
        cy="12"
        r="10"
        stroke="currentColor"
        strokeWidth="4"
      />
      <path
        className="opacity-75"
        fill="currentColor"
        d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
      />
    </svg>
  );
}
