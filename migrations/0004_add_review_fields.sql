-- Add review workflow fields to runs table
-- See spec: specs/daemon-review-api.md Section 9

-- Review status tracks the state of the review workflow
-- Values: pending, reviewed, scrapped, merged, pr_created
ALTER TABLE runs ADD COLUMN review_status TEXT DEFAULT 'pending';

-- Timestamp when review action was taken (Unix epoch milliseconds)
ALTER TABLE runs ADD COLUMN review_action_at INTEGER;

-- URL of the created PR (set when review_status = 'pr_created')
ALTER TABLE runs ADD COLUMN pr_url TEXT;

-- Commit SHA from merge (set when review_status = 'merged')
ALTER TABLE runs ADD COLUMN merge_commit TEXT;
