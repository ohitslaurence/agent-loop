import { useEffect, useCallback, useState } from "react";
import { useNavigate, useLocation } from "@tanstack/react-router";

interface UseKeyboardNavigationOptions {
  items: { id: string }[];
  onSelect: (id: string) => void;
  enabled?: boolean;
}

/**
 * Hook for keyboard navigation in lists.
 * - j/k: move down/up through items
 * - Enter: select current item
 * - Esc: navigate back to home (when on detail page)
 */
export function useKeyboardNavigation({
  items,
  onSelect,
  enabled = true,
}: UseKeyboardNavigationOptions) {
  const [selectedIndex, setSelectedIndex] = useState(-1);

  const handleKeyDown = useCallback(
    (event: KeyboardEvent) => {
      if (!enabled || items.length === 0) return;

      // Ignore if user is typing in an input
      const target = event.target as HTMLElement;
      if (
        target.tagName === "INPUT" ||
        target.tagName === "TEXTAREA" ||
        target.isContentEditable
      ) {
        return;
      }

      switch (event.key) {
        case "j":
          event.preventDefault();
          setSelectedIndex((prev) => Math.min(prev + 1, items.length - 1));
          break;
        case "k":
          event.preventDefault();
          setSelectedIndex((prev) => Math.max(prev - 1, 0));
          break;
        case "Enter":
          event.preventDefault();
          if (selectedIndex >= 0 && selectedIndex < items.length) {
            onSelect(items[selectedIndex].id);
          }
          break;
      }
    },
    [enabled, items, selectedIndex, onSelect]
  );

  useEffect(() => {
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [handleKeyDown]);

  // Reset selection when items change
  useEffect(() => {
    setSelectedIndex(-1);
  }, [items]);

  return {
    selectedIndex,
    setSelectedIndex,
    selectedId: selectedIndex >= 0 ? items[selectedIndex]?.id : null,
  };
}

/**
 * Hook to handle Escape key for navigating back.
 */
export function useEscapeToGoBack(enabled = true) {
  const navigate = useNavigate();
  const location = useLocation();

  const handleKeyDown = useCallback(
    (event: KeyboardEvent) => {
      if (!enabled) return;

      // Ignore if user is typing in an input
      const target = event.target as HTMLElement;
      if (
        target.tagName === "INPUT" ||
        target.tagName === "TEXTAREA" ||
        target.isContentEditable
      ) {
        return;
      }

      if (event.key === "Escape" && location.pathname !== "/") {
        event.preventDefault();
        navigate({ to: "/" });
      }
    },
    [enabled, navigate, location.pathname]
  );

  useEffect(() => {
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [handleKeyDown]);
}
