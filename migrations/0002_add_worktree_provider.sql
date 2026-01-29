-- Add worktree_provider column to runs table
-- See worktrunk-integration.md Section 3.2

ALTER TABLE runs ADD COLUMN worktree_provider TEXT CHECK (worktree_provider IS NULL OR worktree_provider IN ('auto', 'worktrunk', 'git'));
