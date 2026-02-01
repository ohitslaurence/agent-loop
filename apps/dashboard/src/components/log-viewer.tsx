import { useEffect, useRef } from "react";
import { useRunOutput } from "@/hooks/use-run-output";

interface LogViewerProps {
  runId: string;
}

/**
 * Streaming log display with auto-scroll.
 * See spec §2: log-viewer.tsx - streaming log display
 *
 * V0: Simple pre element. Add virtualization in V1 if perf issues arise.
 */
export function LogViewer({ runId }: LogViewerProps) {
  const { output, connected } = useRunOutput(runId);
  const containerRef = useRef<HTMLDivElement>(null);
  const isAtBottomRef = useRef(true);

  // Track if user is at bottom to enable auto-scroll
  const handleScroll = () => {
    const container = containerRef.current;
    if (!container) return;
    const threshold = 50; // px from bottom to consider "at bottom"
    const isAtBottom =
      container.scrollHeight - container.scrollTop - container.clientHeight < threshold;
    isAtBottomRef.current = isAtBottom;
  };

  // Auto-scroll to bottom when new content arrives (if user was at bottom)
  useEffect(() => {
    const container = containerRef.current;
    if (!container || !isAtBottomRef.current) return;
    container.scrollTop = container.scrollHeight;
  }, [output]);

  return (
    <div className="rounded-lg border border-border bg-card">
      <div className="flex items-center justify-between border-b border-border px-3 py-2 sm:px-4">
        <h2 className="font-medium">Output</h2>
        <div className="flex items-center gap-2 text-xs sm:text-sm">
          <span
            className={`h-2 w-2 rounded-full ${
              connected ? "bg-green-500" : "bg-yellow-500 animate-pulse"
            }`}
          />
          <span className="text-muted-foreground">
            {connected ? "Connected" : "Reconnecting..."}
          </span>
        </div>
      </div>
      <div
        ref={containerRef}
        onScroll={handleScroll}
        className="h-64 overflow-auto bg-zinc-950 p-3 sm:h-96 sm:p-4"
      >
        {output ? (
          <pre className="whitespace-pre-wrap break-words font-mono text-xs text-zinc-200 sm:text-sm">
            {output}
          </pre>
        ) : (
          <p className="text-xs text-zinc-500 sm:text-sm">Waiting for output...</p>
        )}
      </div>
    </div>
  );
}
