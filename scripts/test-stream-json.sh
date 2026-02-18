#!/usr/bin/env bash
# Test Claude stream-json output to understand the event format.
# Pipes output to tee so we see it live + capture to a file.
set -euo pipefail

PROMPT="${1:-Say hello in exactly 3 words.}"
OUT="/tmp/claude-stream-json-test.jsonl"

echo "Prompt: $PROMPT"
echo "Output: $OUT"
echo "---"

claude -p --verbose --dangerously-skip-permissions --output-format stream-json --model sonnet "$PROMPT" \
  | tee "$OUT"

echo ""
echo "---"
echo "Lines: $(wc -l < "$OUT")"
echo "Text deltas:"
jq -r 'select(.type == "content_block_delta") | .delta.text // empty' "$OUT" | tr -d '\n'
echo ""
