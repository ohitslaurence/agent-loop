#!/usr/bin/env bash
# spec-picker.sh - Interactive spec discovery and selection
# See: specs/agent-loop-terminal-ux.md §2.1, §3.1, §5.1

# Requires: gum, sourced after agent-loop-ui.sh (for GUM_ENABLED, ui_log)

# -----------------------------------------------------------------------------
# Spec discovery (spec §5.1)
# Discovery rules:
# - Scan specs/*.md excluding specs/README.md and specs/research/
# - Parse first heading as title and optional Status field
# - Compute plan path as specs/planning/<spec>-plan.md
# - Sort by Last Updated descending; fall back to file mtime, then name
# -----------------------------------------------------------------------------

# Parse a spec file and output: spec_path|plan_path|title|spec_status|last_updated
parse_spec_entry() {
  local spec_file="$1"
  local plans_root="${plans_dir:-specs/planning}"
  local spec_title=""
  local spec_status=""
  local spec_last_updated=""

  # Read first 20 lines to find title and metadata
  local content
  content=$(head -20 "$spec_file")

  # Extract first heading (# Title)
  spec_title=$(echo "$content" | grep -m1 '^#[^#]' | sed 's/^#[[:space:]]*//')
  if [[ -z "$spec_title" ]]; then
    spec_title=$(basename "$spec_file" .md)
  fi

  # Extract Status field
  spec_status=$(echo "$content" | grep -m1 '^\*\*Status:\*\*' | sed 's/^\*\*Status:\*\*[[:space:]]*//')

  # Extract Last Updated field (YYYY-MM-DD format)
  spec_last_updated=$(echo "$content" | grep -m1 '^\*\*Last Updated:\*\*' | sed 's/^\*\*Last Updated:\*\*[[:space:]]*//')

  # Compute plan path
  local spec_base
  spec_base=$(basename "$spec_file" .md)
  local computed_plan_path="$plans_root/${spec_base}-plan.md"

  # Output as pipe-delimited record
  printf '%s|%s|%s|%s|%s\n' "$spec_file" "$computed_plan_path" "$spec_title" "$spec_status" "$spec_last_updated"
}

# Discover all spec files and return sorted entries
# Output format: spec_path|plan_path|title|status|last_updated (one per line)
discover_specs() {
  local spec_root="${specs_dir:-specs}"
  local specs=()
  local entries=()

  # Find specs, excluding README.md and research/
  while IFS= read -r -d '' spec_file; do
    # Skip README.md
    if [[ "$(basename "$spec_file")" == "README.md" ]]; then
      continue
    fi
    specs+=("$spec_file")
  done < <(find "$spec_root" -maxdepth 1 -name '*.md' -type f -print0 2>/dev/null)

  if [[ ${#specs[@]} -eq 0 ]]; then
    return 1
  fi

  # Parse each spec
  for spec_file in "${specs[@]}"; do
    local entry
    entry=$(parse_spec_entry "$spec_file")
    entries+=("$entry")
  done

  # Sort entries by last_updated (field 5), then mtime, then name
  # We'll use a decorated-sort-undecorate pattern
  local sorted_entries=()
  while IFS= read -r entry; do
    sorted_entries+=("$entry")
  done < <(
    for entry in "${entries[@]}"; do
      local spec_path last_updated mtime
      spec_path=$(echo "$entry" | cut -d'|' -f1)
      last_updated=$(echo "$entry" | cut -d'|' -f5)

      # Convert last_updated to sortable format, or use mtime
      if [[ -n "$last_updated" && "$last_updated" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]]; then
        # Use last_updated as sort key (YYYY-MM-DD sorts lexicographically)
        printf '%s\t%s\n' "$last_updated" "$entry"
      else
        # Fall back to file mtime
        mtime=$(stat -c '%Y' "$spec_path" 2>/dev/null || stat -f '%m' "$spec_path" 2>/dev/null || echo "0")
        # Convert to sortable date-like format
        printf '%s\t%s\n' "$(date -d "@$mtime" +%Y-%m-%d 2>/dev/null || date -r "$mtime" +%Y-%m-%d 2>/dev/null || echo "0000-00-00")" "$entry"
      fi
    done | sort -t$'\t' -k1 -r | cut -f2
  )

  # Output sorted entries
  for entry in "${sorted_entries[@]}"; do
    printf '%s\n' "$entry"
  done
}

# Format spec entry for display in gum filter
# Input: spec_path|plan_path|title|spec_status|last_updated
# Output: [spec_status] title (last_updated) - filename
format_spec_display() {
  local entry="$1"
  local spec_path spec_title spec_status spec_last_updated

  spec_path=$(echo "$entry" | cut -d'|' -f1)
  spec_title=$(echo "$entry" | cut -d'|' -f3)
  spec_status=$(echo "$entry" | cut -d'|' -f4)
  spec_last_updated=$(echo "$entry" | cut -d'|' -f5)
  local spec_file
  spec_file=$(basename "$spec_path")

  local display=""
  if [[ -n "$spec_status" ]]; then
    display="[$spec_status] "
  fi
  display+="$spec_title"
  if [[ -n "$spec_last_updated" ]]; then
    display+=" ($spec_last_updated)"
  fi
  display+=" - $spec_file"

  printf '%s' "$display"
}

# Interactive spec picker using gum filter
# Returns: sets PICKED_SPEC_PATH and PICKED_PLAN_PATH globals
# Exit code: 0 on success, 1 on cancel/error
spec_picker() {
  # shellcheck disable=SC2034
  PICKED_SPEC_PATH=""
  # shellcheck disable=SC2034
  PICKED_PLAN_PATH=""
  local spec_root="${specs_dir:-specs}"

  if ! check_gum; then
    ui_log "ERROR" "gum is required for spec picker"
    return 1
  fi

  # Discover specs
  local entries=()
  while IFS= read -r entry; do
    entries+=("$entry")
  done < <(discover_specs)

  if [[ ${#entries[@]} -eq 0 ]]; then
    ui_log "ERROR" "No specs found in $spec_root"
    return 1
  fi

  # Build display list and mapping
  local display_lines=()
  declare -A entry_map

  for entry in "${entries[@]}"; do
    local display
    display=$(format_spec_display "$entry")
    display_lines+=("$display")
    entry_map["$display"]="$entry"
  done

  # Run gum filter
  local selected
  selected=$(printf '%s\n' "${display_lines[@]}" | gum filter --placeholder "Select a spec...")

  if [[ -z "$selected" ]]; then
    return 1
  fi

  # Look up the selected entry
  local selected_entry="${entry_map[$selected]}"
  if [[ -z "$selected_entry" ]]; then
    ui_log "ERROR" "Selection lookup failed"
    return 1
  fi

  PICKED_SPEC_PATH=$(echo "$selected_entry" | cut -d'|' -f1)
  PICKED_PLAN_PATH=$(echo "$selected_entry" | cut -d'|' -f2)

  return 0
}

# List known specs (for error messages when gum unavailable)
list_known_specs() {
  printf 'Known specs:\n'
  while IFS= read -r entry; do
    local file_path spec_title
    file_path=$(echo "$entry" | cut -d'|' -f1)
    spec_title=$(echo "$entry" | cut -d'|' -f3)
    printf '  %s - %s\n' "$file_path" "$spec_title"
  done < <(discover_specs)
}
