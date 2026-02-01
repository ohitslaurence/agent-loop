import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/")({
  component: Index,
});

function Index() {
  return (
    <div>
      <h1 className="text-2xl font-bold">Runs</h1>
      <p className="text-muted-foreground">Run list will be displayed here</p>
    </div>
  );
}
