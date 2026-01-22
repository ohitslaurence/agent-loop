#!/usr/bin/env bash
# agent-loop-ui.sh - Gum UI helpers for agent-loop.sh
# See: specs/agent-loop-terminal-ux.md §2, §4.2

# Globals (set by init_ui)
declare -g GUM_ENABLED=false
declare -g LOG_DIR=""
declare -g RUN_ID=""
declare -g RUN_DIR=""
declare -g RUN_LOG=""
declare -g RUN_REPORT=""
declare -g RUN_PROMPT_PATH=""
declare -g RUN_MODEL=""
declare -g RUN_START_MS=""
declare -g PLAN_PATH=""
declare -g PLAN_TASKS_DONE=0
declare -g PLAN_TASKS_TOTAL=0

# Per-iteration stats (spec §3.1 IterationStats)
declare -g ITER_START_MS=""
declare -g ITER_END_MS=""
declare -g ITER_DURATION_MS=""
declare -g ITER_EXIT_CODE=""
declare -g ITER_COMPLETE_DETECTED=""
declare -g ITER_LOG_PATH=""
declare -g ITER_OUTPUT_BYTES=""
declare -g ITER_OUTPUT_LINES=""
declare -g ITER_TAIL_PATH=""

# Aggregated stats
declare -g TOTAL_ITERATIONS=0
declare -g COMPLETED_ITERATION=""
declare -g COMPLETION_MODE=""

# -----------------------------------------------------------------------------
# Gum detection and initialization
# -----------------------------------------------------------------------------

check_gum() {
  command -v gum &>/dev/null
}

init_ui() {
  local log_dir="${1:-logs/agent-loop}"
  local no_gum="${2:-false}"

  # Generate run ID
  RUN_ID=$(date +"%Y%m%d-%H%M%S")
  LOG_DIR="$log_dir"
  RUN_START_MS=$(get_epoch_ms)

  # Create log directory
  if ! mkdir -p "$LOG_DIR"; then
    ui_log "ERROR" "Cannot create log directory: $LOG_DIR"
    return 1
  fi

  RUN_DIR="$LOG_DIR/run-$RUN_ID"

  if ! mkdir -p "$RUN_DIR"; then
    ui_log "ERROR" "Cannot create run directory: $RUN_DIR"
    return 1
  fi

  RUN_LOG="$RUN_DIR/run.log"
  RUN_REPORT="$RUN_DIR/report.tsv"
  RUN_PROMPT_PATH="$RUN_DIR/prompt.txt"

  # Initialize report file
  printf 'timestamp_ms\tkind\titeration\tduration_ms\texit_code\toutput_bytes\toutput_lines\toutput_path\tmessage\ttasks_done\ttasks_total\n' > "$RUN_REPORT"

  # Determine gum availability
  if [[ "$no_gum" == "true" ]]; then
    GUM_ENABLED=false
  elif [[ ! -t 1 ]]; then
    # Non-TTY environment
    GUM_ENABLED=false
  elif check_gum; then
    GUM_ENABLED=true
  else
    GUM_ENABLED=false
  fi

  return 0
}

require_gum() {
  if ! check_gum; then
    cat >&2 <<'EOF'
Error: gum is required but not installed.

Install with:
  brew install gum       # macOS
  go install github.com/charmbracelet/gum@latest

Or run with --no-gum for plain output.
EOF
    return 1
  fi
  return 0
}

# -----------------------------------------------------------------------------
# Timing helpers (spec §3.1)
# -----------------------------------------------------------------------------

get_epoch_ms() {
  # Use date with nanoseconds and truncate to milliseconds
  # Compatible with GNU date (Linux) and BSD date (macOS with coreutils)
  if date +%s%N &>/dev/null; then
    echo $(( $(date +%s%N) / 1000000 ))
  else
    # Fallback to seconds only (macOS without coreutils)
    echo $(( $(date +%s) * 1000 ))
  fi
}

format_duration_ms() {
  local ms="$1"
  local seconds=$((ms / 1000))
  local minutes=$((seconds / 60))
  local remaining_seconds=$((seconds % 60))

  if ((minutes > 0)); then
    printf '%dm %ds' "$minutes" "$remaining_seconds"
  else
    printf '%ds' "$seconds"
  fi
}

# -----------------------------------------------------------------------------
# Report helpers
# -----------------------------------------------------------------------------

sanitize_field() {
  local value="$1"
  value=${value//$'\t'/ }
  value=${value//$'\n'/ }
  value=${value//$'\r'/ }
  printf '%s' "$value"
}

report_event() {
  local kind="$1"
  local iteration="${2:-}"
  local duration_ms="${3:-}"
  local exit_code="${4:-}"
  local output_bytes="${5:-}"
  local output_lines="${6:-}"
  local output_path="${7:-}"
  local message="${8:-}"
  local tasks_done="${9:-${PLAN_TASKS_DONE:-}}"
  local tasks_total="${10:-${PLAN_TASKS_TOTAL:-}}"

  local timestamp_ms
  timestamp_ms=$(get_epoch_ms)

  local safe_message
  safe_message=$(sanitize_field "$message")

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$timestamp_ms" \
    "$kind" \
    "$iteration" \
    "$duration_ms" \
    "$exit_code" \
    "$output_bytes" \
    "$output_lines" \
    "$output_path" \
    "$safe_message" \
    "$tasks_done" \
    "$tasks_total" >> "$RUN_REPORT"
}

refresh_plan_progress() {
  local plan_path="${PLAN_PATH:-}"
  local totals

  if [[ -z "$plan_path" || ! -f "$plan_path" ]]; then
    PLAN_TASKS_DONE=0
    PLAN_TASKS_TOTAL=0
    return 0
  fi

  totals=$(awk '
    /^[[:space:]]*- \[[ xX]\]([[:space:]]|$)/ {total++}
    /^[[:space:]]*- \[[xX]\]([[:space:]]|$)/ {done++}
    END {printf "%d %d", done + 0, total + 0}
  ' "$plan_path")

  PLAN_TASKS_DONE=${totals%% *}
  PLAN_TASKS_TOTAL=${totals##* }
}

write_prompt_snapshot() {
  local prompt="$1"

  printf '%s\n' "$prompt" > "$RUN_PROMPT_PATH"
  ui_log "INFO" "Prompt snapshot written to: $RUN_PROMPT_PATH"
  report_event "PROMPT_SNAPSHOT" "" "" "" "" "" "$RUN_PROMPT_PATH" "prompt"
}

# -----------------------------------------------------------------------------
# Iteration tracking (spec §3.1 IterationStats)
# -----------------------------------------------------------------------------

start_iteration() {
  local iteration="$1"
  refresh_plan_progress
  ITER_START_MS=$(get_epoch_ms)
  ITER_EXIT_CODE=""
  ITER_COMPLETE_DETECTED="false"
  ITER_LOG_PATH="$RUN_DIR/iter-$(printf '%02d' "$iteration").log"
  ITER_OUTPUT_BYTES=""
  ITER_OUTPUT_LINES=""
  ITER_TAIL_PATH="$RUN_DIR/iter-$(printf '%02d' "$iteration").tail.txt"
  TOTAL_ITERATIONS=$iteration

  ui_log "ITERATION_START" "iteration=$iteration"
  report_event "ITERATION_START" "$iteration" "" "" "" "" "" ""

  # Show iteration status
  local elapsed_ms=$((ITER_START_MS - RUN_START_MS))
  local elapsed_str
  elapsed_str=$(format_duration_ms "$elapsed_ms")

  local status_line="Iteration $iteration | Elapsed: $elapsed_str"
  if [[ -n "$ITER_DURATION_MS" ]]; then
    local last_dur
    last_dur=$(format_duration_ms "$ITER_DURATION_MS")
    status_line="$status_line | Last: $last_dur"
  fi
  ui_status "$status_line"
}

end_iteration() {
  local iteration="$1"
  local exit_code="$2"

  ITER_END_MS=$(get_epoch_ms)
  ITER_DURATION_MS=$((ITER_END_MS - ITER_START_MS))
  ITER_EXIT_CODE="$exit_code"
  refresh_plan_progress

  ui_log "ITERATION_END" "iteration=$iteration exit_code=$exit_code duration_ms=$ITER_DURATION_MS"
  report_event "ITERATION_END" "$iteration" "$ITER_DURATION_MS" "$exit_code" "$ITER_OUTPUT_BYTES" "$ITER_OUTPUT_LINES" "$ITER_LOG_PATH" ""
}

record_completion() {
  local iteration="$1"
  local mode="$2"

  ITER_COMPLETE_DETECTED="true"
  COMPLETED_ITERATION="$iteration"
  COMPLETION_MODE="$mode"

  ui_log "COMPLETE_DETECTED" "mode=$mode iteration=$iteration"
  report_event "COMPLETE_DETECTED" "$iteration" "" "" "" "" "$ITER_LOG_PATH" "mode=$mode"
}

# -----------------------------------------------------------------------------
# Claude execution with spinner (spec §5.1)
# -----------------------------------------------------------------------------

run_claude_iteration() {
  local iteration="$1"
  local prompt="$2"
  local model="$3"
  local -n output_ref=$4

  start_iteration "$iteration"

  local temp_output
  temp_output=$(mktemp)
  local exit_code=0

  local pid=""
  local spinner='|/-\\'
  local spinner_index=0

  if [[ "$GUM_ENABLED" == "true" ]]; then
    claude --dangerously-skip-permissions --model "$model" -p "$prompt" > "$temp_output" 2>&1 < /dev/null &
    pid=$!
  else
    # Plain output mode
    printf 'Iteration %d: Running claude...\n' "$iteration"
    claude --dangerously-skip-permissions --model "$model" -p "$prompt" > "$temp_output" 2>&1 < /dev/null &
    pid=$!
  fi

  while kill -0 "$pid" 2>/dev/null; do
    local now_ms
    now_ms=$(get_epoch_ms)
    local run_elapsed_ms=$((now_ms - RUN_START_MS))
    local iter_elapsed_ms=$((now_ms - ITER_START_MS))

    local run_elapsed_str
    run_elapsed_str=$(format_duration_ms "$run_elapsed_ms")
    local iter_elapsed_str
    iter_elapsed_str=$(format_duration_ms "$iter_elapsed_ms")

    local spinner_char=${spinner:spinner_index%4:1}
    spinner_index=$((spinner_index + 1))

    local status_line="$spinner_char Iteration $iteration | Elapsed: $run_elapsed_str | Iter: $iter_elapsed_str"
    if [[ -n "$ITER_DURATION_MS" ]]; then
      local last_dur
      last_dur=$(format_duration_ms "$ITER_DURATION_MS")
      status_line="$status_line | Last: $last_dur"
    fi

    ui_status_inline "$status_line"
    sleep 1
  done

  wait "$pid" || exit_code=$?
  ui_status_done

  output_ref=$(cat "$temp_output")

  ITER_OUTPUT_BYTES=$(wc -c < "$temp_output" | tr -d ' ')
  ITER_OUTPUT_LINES=$(wc -l < "$temp_output" | tr -d ' ')

  local output_warn_threshold=200
  if [[ "$ITER_OUTPUT_BYTES" =~ ^[0-9]+$ ]] && ((ITER_OUTPUT_BYTES > output_warn_threshold)); then
    ui_log "WARN" "Large output (${ITER_OUTPUT_BYTES} bytes) in iteration $iteration"
    report_event "WARN_OUTPUT_LARGE" "$iteration" "" "" "$ITER_OUTPUT_BYTES" "$ITER_OUTPUT_LINES" "$ITER_LOG_PATH" "threshold=$output_warn_threshold"
  fi

  tail -n 200 "$temp_output" > "$ITER_TAIL_PATH"
  local tail_bytes
  tail_bytes=$(wc -c < "$ITER_TAIL_PATH" | tr -d ' ')
  local tail_lines
  tail_lines=$(wc -l < "$ITER_TAIL_PATH" | tr -d ' ')
  report_event "ITERATION_TAIL" "$iteration" "" "" "$tail_bytes" "$tail_lines" "$ITER_TAIL_PATH" "last 200 lines"

  # Write to per-iteration log (spec §4.3)
  cp "$temp_output" "$ITER_LOG_PATH"
  rm -f "$temp_output"

  end_iteration "$iteration" "$exit_code"

  # Warn on empty output
  if [[ -z "$output_ref" ]]; then
    ui_log "WARN" "Empty output from claude in iteration $iteration"
  fi

  # Handle non-zero exit
  if ((exit_code != 0)); then
    ui_log "ERROR" "claude exited with code $exit_code in iteration $iteration"
    ui_log "ERROR" "See iteration log: $ITER_LOG_PATH"
  fi

  return "$exit_code"
}

# -----------------------------------------------------------------------------
# UI helpers (spec §4.2)
# -----------------------------------------------------------------------------

ui_header() {
  local title="$1"
  if [[ "$GUM_ENABLED" == "true" ]]; then
    gum style --border normal --padding "0 1" --border-foreground 212 "$title"
  else
    printf '\n=== %s ===\n\n' "$title"
  fi
}

ui_status() {
  local line="$1"
  if [[ "$GUM_ENABLED" == "true" ]]; then
    gum style --foreground 245 -- "$line"
  else
    printf '%s\n' "$line"
  fi
}

ui_status_inline() {
  local line="$1"
  if [[ ! -t 1 ]]; then
    return
  fi
  if [[ "$GUM_ENABLED" == "true" ]]; then
    local styled
    styled=$(gum style --foreground 245 -- "$line" | tr -d '\n')
    printf '\r'
    tput el 2>/dev/null || true
    printf '%s' "$styled"
  else
    printf '\r'
    tput el 2>/dev/null || true
    printf '%s' "$line"
  fi
}

ui_status_done() {
  if [[ ! -t 1 ]]; then
    return
  fi
  printf '\r'
  tput el 2>/dev/null || true
  printf '\n'
}

ui_spinner() {
  local title="$1"
  shift
  if [[ "$GUM_ENABLED" == "true" ]]; then
    gum spin --spinner dot --title "$title" -- "$@"
  else
    printf '%s... ' "$title"
    "$@"
    printf 'done\n'
  fi
}

ui_log() {
  local level="$1"
  local message="$2"
  local timestamp
  timestamp=$(date +"%Y-%m-%d %H:%M:%S")

  local formatted="[$timestamp] [$level] $message"

  # Write to run log if available
  if [[ -n "$RUN_LOG" ]]; then
    printf '%s\n' "$formatted" >> "$RUN_LOG"
  fi

  # Display based on level and gum availability
  case "$level" in
    ERROR)
      if [[ "$GUM_ENABLED" == "true" ]]; then
        gum style --foreground 196 "$formatted" >&2
      else
        printf '%s\n' "$formatted" >&2
      fi
      ;;
    WARN)
      if [[ "$GUM_ENABLED" == "true" ]]; then
        gum style --foreground 214 "$formatted"
      else
        printf '%s\n' "$formatted"
      fi
      ;;
    INFO)
      if [[ "$GUM_ENABLED" == "true" ]]; then
        gum style --foreground 45 "$formatted"
      else
        printf '%s\n' "$formatted"
      fi
      ;;
    *)
      printf '%s\n' "$formatted"
      ;;
  esac
}

ui_table() {
  local title="$1"
  shift
  local rows=("$@")

  if [[ "$GUM_ENABLED" == "true" ]]; then
    ui_header "$title"
    printf '%s\n' "${rows[@]}" | gum table
  else
    printf '\n--- %s ---\n' "$title"
    printf '%s\n' "${rows[@]}"
    printf '\n'
  fi
}

# -----------------------------------------------------------------------------
# Run header display
# -----------------------------------------------------------------------------

show_run_header() {
  local spec_path="$1"
  local plan_path="$2"
  local iterations="$3"
  local model="$4"

  RUN_MODEL="$model"

  ui_header "Agent Loop"
  ui_status "Run ID:      $RUN_ID"
  ui_status "Spec:        $spec_path"
  ui_status "Plan:        $plan_path"
  ui_status "Iterations:  $iterations"
  ui_status "Model:       $model"
  ui_status "Run dir:     $RUN_DIR"
  ui_status "Gum:         $GUM_ENABLED"

  ui_log "RUN_START" "spec=$spec_path plan=$plan_path iterations=$iterations model=$model"
  report_event "RUN_START" "" "" "" "" "" "" "spec=$spec_path plan=$plan_path iterations=$iterations model=$model"
}

# -----------------------------------------------------------------------------
# Run summary display (spec §3.2 RunSummary, §5.1, §7)
# -----------------------------------------------------------------------------

show_run_summary() {
  local exit_reason="$1"
  local last_exit_code="${2:-0}"

  local run_end_ms
  run_end_ms=$(get_epoch_ms)
  local total_duration_ms=$((run_end_ms - RUN_START_MS))
  local total_duration_str
  total_duration_str=$(format_duration_ms "$total_duration_ms")

  # Calculate average iteration duration
  local avg_duration_ms=0
  local avg_duration_str="N/A"
  if ((TOTAL_ITERATIONS > 0 && total_duration_ms > 0)); then
    avg_duration_ms=$((total_duration_ms / TOTAL_ITERATIONS))
    avg_duration_str=$(format_duration_ms "$avg_duration_ms")
  fi

  ui_log "RUN_END" "reason=$exit_reason iterations=$TOTAL_ITERATIONS total_ms=$total_duration_ms"
  report_event "RUN_END" "$TOTAL_ITERATIONS" "$total_duration_ms" "$last_exit_code" "" "" "" "reason=$exit_reason"

  # Build summary rows
  local -a rows=(
    "Metric,Value"
    "Run ID,$RUN_ID"
    "Exit Reason,$exit_reason"
    "Iterations Run,$TOTAL_ITERATIONS"
    "Total Duration,$total_duration_str"
    "Avg Iteration,$avg_duration_str"
    "Last Exit Code,$last_exit_code"
    "Model,$RUN_MODEL"
    "Run Log,$RUN_LOG"
    "Run Report,$RUN_REPORT"
    "Prompt Snapshot,$RUN_PROMPT_PATH"
  )

  if [[ -n "$COMPLETED_ITERATION" ]]; then
    rows+=("Completed Iteration,$COMPLETED_ITERATION")
    rows+=("Completion Mode,$COMPLETION_MODE")
  fi

  if [[ -n "$ITER_LOG_PATH" ]]; then
    rows+=("Last Iteration Log,$ITER_LOG_PATH")
  fi

  if [[ -n "$ITER_TAIL_PATH" ]]; then
    rows+=("Last Output Tail,$ITER_TAIL_PATH")
  fi

  # Display summary
  if [[ "$GUM_ENABLED" == "true" ]]; then
    printf '\n'
    gum style --border double --padding "0 1" --border-foreground 212 "Run Summary"
    printf '%s\n' "${rows[@]}" | gum table --separator ","
  else
    printf '\n=== Run Summary ===\n'
    for row in "${rows[@]:1}"; do
      local key="${row%%,*}"
      local val="${row#*,}"
      printf '%-20s %s\n' "$key:" "$val"
    done
    printf '\n'
  fi

  # Warning emitted before RUN_END for completion violations.
}

# -----------------------------------------------------------------------------
# Summary JSON output (spec §3.2, §4.1)
# -----------------------------------------------------------------------------

write_summary_json() {
  local exit_reason="$1"
  local last_exit_code="${2:-0}"

  local run_end_ms
  run_end_ms=$(get_epoch_ms)
  local total_duration_ms=$((run_end_ms - RUN_START_MS))

  # Calculate average iteration duration
  local avg_duration_ms=0
  if ((TOTAL_ITERATIONS > 0 && total_duration_ms > 0)); then
    avg_duration_ms=$((total_duration_ms / TOTAL_ITERATIONS))
  fi

  local summary_path="$RUN_DIR/summary.json"

  # Format nullable fields properly for JSON
  local completed_iter_json="null"
  [[ -n "$COMPLETED_ITERATION" ]] && completed_iter_json="$COMPLETED_ITERATION"

  local completion_mode_json="null"
  [[ -n "$COMPLETION_MODE" ]] && completion_mode_json="\"$COMPLETION_MODE\""

  local last_iter_log_json="null"
  [[ -n "$ITER_LOG_PATH" ]] && last_iter_log_json="\"$ITER_LOG_PATH\""

  local last_iter_tail_json="null"
  [[ -n "$ITER_TAIL_PATH" ]] && last_iter_tail_json="\"$ITER_TAIL_PATH\""

  # Build JSON (avoiding jq dependency)
  cat > "$summary_path" <<EOF
{
  "run_id": "$RUN_ID",
  "start_ms": $RUN_START_MS,
  "end_ms": $run_end_ms,
  "total_duration_ms": $total_duration_ms,
  "iterations_run": $TOTAL_ITERATIONS,
  "completed_iteration": $completed_iter_json,
  "avg_duration_ms": $avg_duration_ms,
  "last_exit_code": $last_exit_code,
  "completion_mode": $completion_mode_json,
  "model": "$RUN_MODEL",
  "exit_reason": "$exit_reason",
  "run_log": "$RUN_LOG",
  "run_report": "$RUN_REPORT",
  "prompt_snapshot": "$RUN_PROMPT_PATH",
  "last_iteration_tail": $last_iter_tail_json,
  "last_iteration_log": $last_iter_log_json
}
EOF

  ui_log "INFO" "Summary JSON written to: $summary_path"
}

# -----------------------------------------------------------------------------
# Completion screen with optional wait (spec §4.1, §5.1)
# -----------------------------------------------------------------------------

show_completion_screen() {
  local no_wait="${1:-false}"

  if [[ "$no_wait" == "true" ]]; then
    return 0
  fi

  if [[ "$GUM_ENABLED" == "true" ]]; then
    # Show styled completion message and wait for user confirmation (spec §4.1)
    printf '\n'
    gum style --foreground 46 --bold "✓ Agent loop finished"
    gum confirm --default=true "Close" || true
  else
    # Plain text fallback
    printf '\n✓ Agent loop finished\n'
    printf 'Press Enter to close...'
    read -r
  fi
}

# -----------------------------------------------------------------------------
# Signal handling (spec §2.1, §5.2)
# -----------------------------------------------------------------------------

# Track whether cleanup has already run to avoid double-execution
declare -g CLEANUP_DONE=false

# Store signal info for proper exit codes
declare -g SIGNAL_RECEIVED=""

cleanup_on_signal() {
  local signal="${1:-EXIT}"

  # Avoid running cleanup twice
  if [[ "$CLEANUP_DONE" == "true" ]]; then
    return
  fi
  CLEANUP_DONE=true

  SIGNAL_RECEIVED="$signal"

  # Only print summary if we've started running (RUN_START_MS is set)
  if [[ -n "$RUN_START_MS" ]]; then
    case "$signal" in
      INT)
        ui_log "INFO" "Received SIGINT - interrupting"
        show_run_summary "interrupted_sigint" "130"
        ;;
      TERM)
        ui_log "INFO" "Received SIGTERM - terminating"
        show_run_summary "interrupted_sigterm" "143"
        ;;
      EXIT)
        # Normal exit - don't show summary here, it's handled by main flow
        ;;
    esac
  fi

  # Restore terminal state if gum was being used
  if [[ "$GUM_ENABLED" == "true" ]]; then
    # Reset any gum spinner or style that might be in progress
    tput cnorm 2>/dev/null || true  # Show cursor
    printf '\033[0m' 2>/dev/null || true  # Reset colors
  fi
}

setup_signal_traps() {
  trap 'cleanup_on_signal INT; exit 130' INT
  trap 'cleanup_on_signal TERM; exit 143' TERM
  trap 'cleanup_on_signal EXIT' EXIT
}
