import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { RouterProvider, createRouter } from "@tanstack/react-router";
import {
  QueryClient,
  QueryClientProvider,
  QueryCache,
  MutationCache,
} from "@tanstack/react-query";
import { toast } from "sonner";
import "./index.css";

// Import the generated route tree
import { routeTree } from "./routeTree.gen";

/**
 * Show toast for transient errors.
 * See spec §6: "Add toast notifications for transient errors"
 */
function handleQueryError(error: Error) {
  // Don't toast for 404s - those are expected for missing resources
  if (error.message.includes("404") || error.message.includes("not found")) {
    return;
  }
  // Don't toast for daemon unavailable - that's handled by the banner
  if (error.message.includes("Failed to fetch")) {
    return;
  }
  toast.error(error.message);
}

// Create query client with global error handling
const queryClient = new QueryClient({
  queryCache: new QueryCache({
    onError: handleQueryError,
  }),
  mutationCache: new MutationCache({
    onError: handleQueryError,
  }),
  defaultOptions: {
    queries: {
      staleTime: 5000, // 5s before refetch on mount
      refetchOnWindowFocus: true,
    },
  },
});

// Create router instance
const router = createRouter({ routeTree });

// Register router for type safety
declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  </StrictMode>
);
