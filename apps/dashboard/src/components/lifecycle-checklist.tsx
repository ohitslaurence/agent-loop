import type { Run, RunEvent, LifecycleStep, Step, StepPhase } from "@/lib/types";

interface LifecycleChecklistProps {
  run: Run;
  events: RunEvent[];
  steps?: Step[];
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
export function LifecycleChecklist({ run, events, steps }: LifecycleChecklistProps) {
  const lifecycleSteps = deriveLifecycleSteps(run, events, steps ?? []);

  if (lifecycleSteps.length === 0) {
    return null;
  }

  return (
    <div className="rounded-lg border border-border bg-card">
      <div className="border-b border-border px-4 py-2">
        <h2 className="font-medium">Lifecycle</h2>
      </div>
      <ul className="divide-y divide-border">
        {lifecycleSteps.map((step, index) => (
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

function deriveLifecycleSteps(run: Run, events: RunEvent[], steps: Step[]): LifecycleStep[] {
  const lifecycleSteps: LifecycleStep[] = [];

  // Helper to select the latest step per phase (highest attempt)
  const stepsByPhase = new Map<StepPhase, Step>();
  for (const step of steps) {
    const existing = stepsByPhase.get(step.phase);
    if (!existing || step.attempt > existing.attempt) {
      stepsByPhase.set(step.phase, step);
    }
  }

  // Helper to match event types case-insensitively
  const matchType = (e: RunEvent, type: string) =>
    e.event_type.toLowerCase() === type.toLowerCase();

  // Run started
  const startEvent = events.find((e) => matchType(e, "RUN_STARTED"));
  const hasStarted = !!startEvent || run.status !== "Pending";
  lifecycleSteps.push({
    label: "Run started",
    completed: hasStarted,
    inProgress: false,
    timestamp: startEvent
      ? new Date(startEvent.timestamp).toISOString()
      : hasStarted
      ? run.created_at
      : undefined,
  });

  const implSteps = steps.filter((step) => step.phase === "Implementation");
  const implIterations = implSteps.length;
  const implLatest = stepsByPhase.get("Implementation");
  const implInProgress = implLatest?.status === "Running";
  lifecycleSteps.push({
    label:
      implIterations > 0
        ? `Implementation (${implIterations} iteration${implIterations > 1 ? "s" : ""})`
        : "Implementation",
    completed: implLatest?.status === "Succeeded",
    inProgress: implInProgress,
    timestamp: implLatest?.completed_at ?? implLatest?.started_at,
  });

  // Self-review completed
  const reviewStep = stepsByPhase.get("Review");
  lifecycleSteps.push({
    label: "Self-review (automated)",
    completed: reviewStep?.status === "Succeeded",
    inProgress: reviewStep?.status === "Running",
    timestamp: reviewStep?.completed_at ?? reviewStep?.started_at,
  });

  // Verification completed
  const verifyStep = stepsByPhase.get("Verification");
  lifecycleSteps.push({
    label: "Verification",
    completed: verifyStep?.status === "Succeeded",
    inProgress: verifyStep?.status === "Running",
    timestamp: verifyStep?.completed_at ?? verifyStep?.started_at,
  });

  // Worktree merged/removed
  const worktreeRemoved = events.find((e) => matchType(e, "WORKTREE_REMOVED"));
  lifecycleSteps.push({
    label: "Worktree cleaned up",
    completed: !!worktreeRemoved,
    inProgress: false,
    timestamp: worktreeRemoved ? new Date(worktreeRemoved.timestamp).toISOString() : undefined,
  });

  // Run completed
  const runCompleted = events.find((e) => matchType(e, "RUN_COMPLETED"));
  const isCompleted = run.status === "Completed";
  lifecycleSteps.push({
    label: "Run completed",
    completed: isCompleted || !!runCompleted,
    inProgress: run.status === "Running",
    timestamp: runCompleted
      ? new Date(runCompleted.timestamp).toISOString()
      : isCompleted
      ? run.updated_at
      : undefined,
  });

  // Branch ready for review (final state for completed runs with branch)
  const branchReady = run.status === "Completed" && run.worktree?.run_branch;
  lifecycleSteps.push({
    label: "Branch ready for review",
    completed: !!branchReady,
    inProgress: false,
  });

  return lifecycleSteps;
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
