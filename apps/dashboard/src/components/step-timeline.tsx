import type { Step, StepPhase, StepStatus, RunStatus } from "@/lib/types";

const phaseOrder: StepPhase[] = [
  "Implementation",
  "Review",
  "Verification",
  "Watchdog",
  "Merge",
];

const statusStyles: Record<StepStatus, { bg: string; icon: string; animate?: string }> = {
  Pending: {
    bg: "bg-muted",
    icon: "text-muted-foreground",
  },
  Running: {
    bg: "bg-blue-500",
    icon: "text-white",
    animate: "animate-pulse ring-4 ring-blue-500/30",
  },
  Succeeded: {
    bg: "bg-green-500",
    icon: "text-white",
  },
  Failed: {
    bg: "bg-red-500",
    icon: "text-white",
  },
};

function formatTime(iso: string): string {
  try {
    return new Date(iso).toLocaleTimeString();
  } catch {
    return iso;
  }
}

function formatDuration(startIso: string, endIso: string): string {
  try {
    const start = new Date(startIso).getTime();
    const end = new Date(endIso).getTime();
    const seconds = Math.floor((end - start) / 1000);
    if (seconds < 60) return `${seconds}s`;
    const minutes = Math.floor(seconds / 60);
    const remainingSeconds = seconds % 60;
    return `${minutes}m ${remainingSeconds}s`;
  } catch {
    return "";
  }
}

function StatusIcon({ status }: { status: StepStatus }) {
  switch (status) {
    case "Succeeded":
      return (
        <svg className="h-3 w-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={3}>
          <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
        </svg>
      );
    case "Failed":
      return (
        <svg className="h-3 w-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={3}>
          <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
        </svg>
      );
    case "Running":
      return (
        <svg className="h-3 w-3 animate-spin" fill="none" viewBox="0 0 24 24">
          <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
          <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
        </svg>
      );
    case "Pending":
    default:
      return <div className="h-2 w-2 rounded-full bg-current" />;
  }
}

interface StepTimelineProps {
  steps: Step[];
  runStatus?: RunStatus;
}

export function StepTimeline({ steps, runStatus }: StepTimelineProps) {
  // Group steps by phase, keeping the latest attempt for each phase
  const stepsByPhase = new Map<StepPhase, Step>();
  for (const step of steps) {
    const existing = stepsByPhase.get(step.phase);
    if (!existing || step.attempt > existing.attempt) {
      stepsByPhase.set(step.phase, step);
    }
  }

  // Build timeline entries in phase order
  const timeline = phaseOrder.map((phase) => {
    const step = stepsByPhase.get(phase);
    return { phase, step };
  });

  // Filter out phases that haven't been reached yet (no step and all previous phases completed or not started)
  const lastActiveIndex = timeline.findIndex(
    (t) => t.step?.status === "Running" || t.step?.status === "Failed"
  );
  const lastCompletedIndex = timeline.reduce(
    (acc, t, i) => (t.step?.status === "Succeeded" ? i : acc),
    -1
  );
  const displayUpTo = Math.max(
    lastActiveIndex,
    lastCompletedIndex,
    timeline.findIndex((t) => t.step !== undefined)
  );

  // Show at least up to the next pending phase after the last active
  const isTerminal =
    runStatus === "Completed" || runStatus === "Failed" || runStatus === "Canceled";

  const visibleTimeline = isTerminal
    ? timeline.filter((t) => t.step)
    : timeline.filter((t, i) => {
        if (t.step) return true;
        // Show next phase after last step as pending
        if (i === displayUpTo + 1 && displayUpTo >= 0) return true;
        return false;
      });

  if (visibleTimeline.length === 0) {
    return (
      <div className="rounded-lg border border-border bg-card p-4">
        <h2 className="mb-3 font-medium">Steps</h2>
        <p className="text-sm text-muted-foreground">No steps yet</p>
      </div>
    );
  }

  return (
    <div className="rounded-lg border border-border bg-card p-4">
      <h2 className="mb-4 font-medium">Steps</h2>
      <div className="relative">
        {visibleTimeline.map((entry, index) => {
          const step = entry.step;
          const status: StepStatus = step?.status ?? "Pending";
          const styles = statusStyles[status];
          const isLast = index === visibleTimeline.length - 1;

          return (
            <div key={entry.phase} className="relative flex gap-3 pb-4 last:pb-0">
              {/* Connector line */}
              {!isLast && (
                <div className="absolute left-[11px] top-6 h-full w-0.5 bg-border" />
              )}

              {/* Status indicator */}
              <div
                className={`relative z-10 flex h-6 w-6 shrink-0 items-center justify-center rounded-full ${styles.bg} ${styles.icon} ${styles.animate ?? ""}`}
              >
                <StatusIcon status={status} />
              </div>

              {/* Content */}
              <div className="min-w-0 flex-1">
                <div className="flex items-baseline justify-between gap-2">
                  <span className="font-medium">{entry.phase}</span>
                  {step?.attempt && (
                    <span className="text-xs text-muted-foreground">
                      Iteration {step.attempt}
                    </span>
                  )}
                </div>

                {step && (
                  <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
                    {step.started_at && (
                      <span>Started {formatTime(step.started_at)}</span>
                    )}
                    {step.started_at && step.completed_at && (
                      <span>
                        Duration: {formatDuration(step.started_at, step.completed_at)}
                      </span>
                    )}
                    {step.exit_code !== undefined && (
                      <span className={step.exit_code === 0 ? "text-green-600 dark:text-green-400" : "text-red-500 dark:text-red-400"}>
                        Exit {step.exit_code}
                      </span>
                    )}
                  </div>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
