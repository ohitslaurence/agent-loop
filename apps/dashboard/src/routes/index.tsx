import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { useRuns } from "@/hooks/use-runs";
import { RunList } from "@/components/run-list";
import { WorkspaceSwitcher } from "@/components/workspace-switcher";

interface SearchParams {
  workspace?: string;
}

export const Route = createFileRoute("/")({
  component: Index,
  validateSearch: (search: Record<string, unknown>): SearchParams => {
    return {
      workspace:
        typeof search.workspace === "string" ? search.workspace : undefined,
    };
  },
});

function Index() {
  const { workspace } = Route.useSearch();
  const navigate = useNavigate();
  const { data: runs, isLoading, error } = useRuns(workspace);

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

  const handleWorkspaceChange = (newWorkspace: string | null) => {
    navigate({
      to: "/",
      search: newWorkspace ? { workspace: newWorkspace } : {},
    });
  };

  // Need all runs to derive workspace list for switcher
  const { data: allRuns } = useRuns();

  if (!runs || runs.length === 0) {
    return (
      <div className="py-12 text-center">
        <div className="mb-6 flex items-center justify-between">
          <h1 className="text-2xl font-bold">Runs</h1>
          {allRuns && (
            <WorkspaceSwitcher
              runs={allRuns}
              selectedWorkspace={workspace ?? null}
              onWorkspaceChange={handleWorkspaceChange}
            />
          )}
        </div>
        <p className="text-muted-foreground">No runs found</p>
      </div>
    );
  }

  return (
    <div>
      <div className="mb-6 flex items-center justify-between">
        <h1 className="text-2xl font-bold">Runs</h1>
        <WorkspaceSwitcher
          runs={allRuns ?? runs}
          selectedWorkspace={workspace ?? null}
          onWorkspaceChange={handleWorkspaceChange}
        />
      </div>
      <RunList runs={runs} />
    </div>
  );
}
