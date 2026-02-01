import { createFileRoute } from "@tanstack/react-router";
import { useRuns } from "@/hooks/use-runs";
import { RunCard } from "@/components/run-card";

export const Route = createFileRoute("/")({
  component: Index,
});

function Index() {
  const { data: runs, isLoading, error } = useRuns();

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-12">
        <div className="text-muted-foreground">Loading runs...</div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="rounded-md border border-destructive/50 bg-destructive/10 p-4">
        <p className="text-destructive">Failed to load runs: {error.message}</p>
        <button
          onClick={() => window.location.reload()}
          className="mt-2 text-sm underline"
        >
          Retry
        </button>
      </div>
    );
  }

  if (!runs || runs.length === 0) {
    return (
      <div className="py-12 text-center">
        <h1 className="text-2xl font-bold">Runs</h1>
        <p className="mt-2 text-muted-foreground">No runs found</p>
      </div>
    );
  }

  return (
    <div>
      <h1 className="mb-6 text-2xl font-bold">Runs</h1>
      <div className="grid gap-4">
        {runs.map((run) => (
          <RunCard key={run.id} run={run} />
        ))}
      </div>
    </div>
  );
}
