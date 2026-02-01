import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { Run } from "@/lib/types";

interface WorkspaceSwitcherProps {
  runs: Run[];
  selectedWorkspace: string | null;
  onWorkspaceChange: (workspace: string | null) => void;
}

/**
 * Dropdown to filter runs by workspace.
 * Derives unique workspaces from run list per spec §5 (Workspace Switching).
 */
export function WorkspaceSwitcher({
  runs,
  selectedWorkspace,
  onWorkspaceChange,
}: WorkspaceSwitcherProps) {
  // Extract unique workspace_root values from runs
  const workspaces = [...new Set(runs.map((r) => r.workspace_root))].sort();

  if (workspaces.length <= 1) {
    // No point showing switcher with only one workspace
    return null;
  }

  // Display-friendly name (last path segment)
  const displayName = (workspace: string) =>
    workspace.split("/").pop() ?? workspace;

  return (
    <Select
      value={selectedWorkspace ?? "all"}
      onValueChange={(value) =>
        onWorkspaceChange(value === "all" ? null : value)
      }
    >
      <SelectTrigger className="w-full sm:w-[180px]">
        <SelectValue placeholder="All workspaces" />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value="all">All workspaces</SelectItem>
        {workspaces.map((workspace) => (
          <SelectItem key={workspace} value={workspace}>
            {displayName(workspace)}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
