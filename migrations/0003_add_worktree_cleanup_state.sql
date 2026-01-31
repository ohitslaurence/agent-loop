-- Add worktree cleanup tracking to runs table
-- Tracks cleanup status and cleanup timestamp

ALTER TABLE runs ADD COLUMN worktree_cleanup_status TEXT;
ALTER TABLE runs ADD COLUMN worktree_cleaned_at INTEGER;
