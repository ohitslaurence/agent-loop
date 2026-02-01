import { useNavigate } from "@tanstack/react-router";
import { useCallback, useEffect, useRef } from "react";
import type { Run } from "@/lib/types";
import { RunCard } from "./run-card";
import { useKeyboardNavigation } from "@/hooks/use-keyboard-navigation";

interface RunListProps {
  runs: Run[];
}

export function RunList({ runs }: RunListProps) {
  const navigate = useNavigate();
  const listRef = useRef<HTMLDivElement>(null);

  const handleSelect = useCallback(
    (id: string) => {
      navigate({ to: "/runs/$runId", params: { runId: id } });
    },
    [navigate]
  );

  const { selectedIndex, selectedId } = useKeyboardNavigation({
    items: runs,
    onSelect: handleSelect,
  });

  // Scroll selected item into view
  useEffect(() => {
    if (selectedIndex >= 0 && listRef.current) {
      const items = listRef.current.querySelectorAll("[data-run-card]");
      const selectedItem = items[selectedIndex] as HTMLElement | undefined;
      selectedItem?.scrollIntoView({ block: "nearest", behavior: "smooth" });
    }
  }, [selectedIndex]);

  return (
    <div ref={listRef} className="grid gap-4">
      {runs.map((run) => (
        <RunCard key={run.id} run={run} isSelected={run.id === selectedId} />
      ))}
      {runs.length > 0 && (
        <p className="text-xs text-muted-foreground">
          Tip: Use <kbd className="rounded bg-muted px-1">j</kbd>/
          <kbd className="rounded bg-muted px-1">k</kbd> to navigate,{" "}
          <kbd className="rounded bg-muted px-1">Enter</kbd> to select
        </p>
      )}
    </div>
  );
}
