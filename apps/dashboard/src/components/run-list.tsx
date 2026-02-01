import type { Run } from "@/lib/types";
import { RunCard } from "./run-card";

interface RunListProps {
  runs: Run[];
}

export function RunList({ runs }: RunListProps) {
  return (
    <div className="grid gap-4">
      {runs.map((run) => (
        <RunCard key={run.id} run={run} />
      ))}
    </div>
  );
}
