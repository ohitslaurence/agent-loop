import { createFileRoute, Link } from "@tanstack/react-router";
import { useRun } from "@/hooks/use-run";
import { RunDetail as RunDetailComponent } from "@/components/run-detail";

export const Route = createFileRoute("/runs/$runId")({
  component: RunDetailPage,
});

function RunDetailPage() {
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

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-4">
        <Link to="/" className="text-sm text-muted-foreground hover:underline">
          &larr; Back to runs
        </Link>
      </div>

      <RunDetailComponent run={run} />

      {/* Step timeline will be added when step-timeline.tsx is created */}
    </div>
  );
}
