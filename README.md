# loop

A CLI tool that runs Claude Code in an autonomous loop to implement spec-driven tasks. Give it a spec and a plan, and it works through each task one commit at a time until everything is done.

## How It Works

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│    Spec     │     │    Plan     │     │   Claude    │
│  (what to   │ ──▶ │  (checklist │ ──▶ │   Code      │ ──┐
│   build)    │     │  of tasks)  │     │             │   │
└─────────────┘     └─────────────┘     └─────────────┘   │
                                                          │
       ┌──────────────────────────────────────────────────┘
       │
       ▼
┌─────────────────────────────────────────────────────────┐
│  Loop iteration:                                        │
│  1. Claude reads spec + plan                            │
│  2. Picks ONE unchecked task                            │
│  3. Implements it                                       │
│  4. Marks task [x] in plan                              │
│  5. Commits with gritty                                 │
│  6. If all done → outputs <promise>COMPLETE</promise>   │
│     Otherwise → next iteration                          │
└─────────────────────────────────────────────────────────┘
```

The loop continues until Claude outputs `<promise>COMPLETE</promise>` or hits the iteration limit.

## Installation

```bash
git clone git@github.com:ohitslaurence/agent-loop.git ~/dev/personal/agent-loop
cd ~/dev/personal/agent-loop
./install.sh
```

This symlinks `loop` and `loop-analyze` to `~/.local/bin/`.

For system-wide install: `./install.sh --global` (uses `/usr/local/bin/`, requires sudo).

### Dependencies

| Dependency | Required | Purpose |
|------------|----------|---------|
| [Claude Code CLI](https://docs.anthropic.com/en/docs/claude-code) | Yes | The AI agent that does the work |
| [gritty](https://github.com/ohitslaurence/gritty) | Yes* | AI-powered git commits (referenced in default prompt) |
| [gum](https://github.com/charmbracelet/gum) | No | Interactive spec picker, styled terminal output |

*You can use a custom prompt that doesn't require gritty.

## Quick Start

```bash
# 1. Set up your project
cd your-project
loop --init-config

# 2. Create a spec
cat > specs/user-auth.md << 'EOF'
# User Authentication

**Status:** Draft
**Last Updated:** 2025-01-22

## Overview
Add basic username/password authentication to the app.

## Requirements
- Login form with username and password fields
- Session management with secure cookies
- Logout endpoint that clears session
- Protected routes that require authentication
EOF

# 3. Create a plan
cat > specs/planning/user-auth-plan.md << 'EOF'
# User Auth Implementation Plan

## Tasks
- [ ] Create User model with password hashing
- [ ] Add login API endpoint
- [ ] Add logout API endpoint
- [ ] Create login form component
- [ ] Add session middleware
- [ ] Protect dashboard routes
- [ ] Add "logged in as" indicator to header

## Verification
- [ ] Can register new user
- [ ] Can login with valid credentials
- [ ] Invalid credentials show error
- [ ] Logout clears session
- [ ] Protected routes redirect to login
EOF

# 4. Run the loop
loop specs/user-auth.md
```

## Usage

```
loop [command] [spec-path] [plan-path] [options]
```

### Commands

| Command | Description |
|---------|-------------|
| (none) | Run the agent loop (default) |
| `prompt` | Show the prompt that would be sent to Claude, then exit |

```bash
# Preview the prompt without running
loop prompt specs/my-feature.md

# Run the loop
loop specs/my-feature.md
```

### Arguments

| Argument | Description |
|----------|-------------|
| `spec-path` | Path to the spec file. Optional if gum is available (shows interactive picker). |
| `plan-path` | Path to the plan file. Defaults to `<plans_dir>/<spec-name>-plan.md`. |

### Options

| Option | Default | Description |
|--------|---------|-------------|
| `--iterations <n>` | 50 | Maximum loop iterations before stopping |
| `--model <name>` | opus | Claude model to use (opus, sonnet, haiku) |
| `--log-dir <path>` | logs/loop | Where to write run logs |
| `--completion-mode` | trailing | How to detect completion (see below) |
| `--mode <name>` | plan | Run mode (plan or experiment) |
| `--prompt <path>` | - | Custom prompt file (overrides `.loop/prompt.txt`) |
| `--verify-cmd <cmd>` | - | Verification command to run after each iteration (repeatable) |
| `--verify-timeout-sec <n>` | 0 | Timeout per verification command (0 = none) |
| `--measure-cmd <cmd>` | - | Measurement command (experiment mode). Writes to `LOOP_METRICS_OUT` |
| `--measure-timeout-sec <n>` | 0 | Timeout per measurement command (0 = none) |
| `--claude-timeout-sec <n>` | 0 | Timeout per Claude iteration (0 = none) |
| `--claude-retries <n>` | 0 | Retries per iteration on non-zero exit |
| `--claude-retry-backoff-sec <n>` | 5 | Seconds to sleep between retries |
| `--no-postmortem` | - | Skip the post-run analysis |
| `--no-gum` | - | Disable gum UI, use plain output |
| `--no-wait` | - | Don't wait for keypress at completion |
| `--config <path>` | - | Load specific config file |
| `--init-config` | - | Create `.loop/config` in current project |

### Interactive Spec Picker

If you run `loop` without arguments and gum is installed, you get an interactive picker:

```
$ loop
? Select a spec...
> [Draft] User Authentication (2025-01-22) - user-auth.md
  [In Progress] API Rate Limiting (2025-01-20) - rate-limiting.md
  [Complete] Database Schema (2025-01-15) - db-schema.md
```

The picker scans `specs/*.md`, extracts metadata from each file, and sorts by last updated date.

## Project Configuration

Run `loop --init-config` to create `.loop/config`:

```ini
# Directories
specs_dir="specs"
plans_dir="specs/planning"
log_dir="logs/loop"

# Execution
model="opus"
iterations=50
completion_mode="trailing"
mode="plan"

# Verification (optional)
# Use | to separate multiple commands.
# You can also pass --verify-cmd multiple times.
# verify_cmds="bun test|bun lint"
verify_cmds=""
verify_timeout_sec=0
measure_cmd=""
measure_timeout_sec=0

# Resiliency (optional)
claude_timeout_sec=0
claude_retries=0
claude_retry_backoff_sec=5

# Features
postmortem=true
summary_json=true
no_wait=false
no_gum=false

# Custom prompt (optional)
# prompt_file=".loop/prompt.txt"

# Additional context files to include in prompt (optional)
# context_files="specs/README.md specs/planning/SPEC_AUTHORING.md CLAUDE.md"
```

## Verification Commands

If you configure verification commands, loop runs them after each successful agent iteration.
If verification fails, loop writes a failure context into the run logs and instructs the next
iteration to fix verification before advancing the plan.

Configure via config:

```ini
verify_cmds="bun test|bun lint"
verify_timeout_sec=600
```

Or via CLI:

```bash
loop specs/my-feature.md --verify-cmd "bun test" --verify-cmd "bun lint"
```

Note: command timeouts require `timeout` on PATH (commonly from GNU coreutils).

## Experiment Mode

Experiment mode runs iterative attempts toward a goal instead of a checklist. It captures metrics
per iteration and writes an experiment log that subsequent agents can read.

Configure measurement with `measure_cmd`, which should write metrics to the path provided by
`LOOP_METRICS_OUT`:

```ini
mode="experiment"
verify_cmds="npm run test:playwright"
measure_cmd="node scripts/measure-bundle.js --out $LOOP_METRICS_OUT"
measure_timeout_sec=120
```

The runner exports:

- `LOOP_METRICS_OUT` - file path for metrics output
- `LOOP_ITERATION` - current iteration number
- `LOOP_RUN_DIR` - run directory
- `LOOP_SPEC_PATH` - spec path

Artifacts are saved under `logs/loop/run-<id>/`:

- `metrics/iter-XX.json`
- `summaries/iter-XX.md`
- `experiment-log.md`

### Context Files

The `context_files` option lets you include additional files as `@path` references in the prompt. This is useful for:

- **Spec writing guidelines** - How specs should be structured
- **Coding standards** - Project-specific conventions
- **Architecture docs** - Context about the codebase
- **CLAUDE.md** - Instructions for Claude

```ini
context_files="specs/README.md specs/planning/SPEC_AUTHORING.md CLAUDE.md"
```

This generates a prompt starting with:
```
@specs/feature.md @specs/planning/feature-plan.md @specs/README.md @specs/planning/SPEC_AUTHORING.md @CLAUDE.md
```

## Custom Prompts

Create `.loop/prompt.txt` to customize the agent's behavior. Use these placeholders:

| Placeholder | Replaced With |
|-------------|---------------|
| `SPEC_PATH` | Path to the spec file |
| `PLAN_PATH` | Path to the plan file |

### Example Custom Prompt

```
@SPEC_PATH @PLAN_PATH @docs/ARCHITECTURE.md

You are an implementation agent working on a TypeScript/React codebase.

## Your Task
1. Read the spec and plan carefully
2. Pick ONE unchecked `[ ]` task from the plan
3. Implement it following our coding standards
4. Mark the task `[x]` when complete
5. Run `bun test` to verify
6. Commit using `gritty commit --accept`

## When You're Done
If ALL tasks are checked `[x]`, output exactly:
<promise>COMPLETE</promise>

Otherwise, output ONE line: "Completed [task name]. [N] tasks remain."

## Rules
- One task per iteration
- Don't modify unrelated code
- Don't skip tests
- Use existing patterns from the codebase
```

### Default Prompt

The built-in prompt instructs Claude to:

1. Pick the highest-priority unchecked task
2. Implement only that task
3. Run relevant verification steps
4. Update the plan checklist
5. Make one atomic commit via gritty
6. Output `<promise>COMPLETE</promise>` when all tasks are done

It also includes guardrails for spec alignment, schema matching, and handling ambiguity.

## Completion Detection

The loop watches for `<promise>COMPLETE</promise>` in Claude's output.

| Mode | Behavior |
|------|----------|
| `exact` | Entire response must be exactly `<promise>COMPLETE</promise>` |
| `trailing` (default) | Token must be the last non-empty line |

The `trailing` mode is more forgiving—Claude can include a brief message before the token.

## Logs and Reports

Each run creates a directory: `logs/loop/run-<YYYYMMDD-HHMMSS>/`

```
logs/loop/run-20250122-143052/
├── run.log              # Human-readable event log
├── report.tsv           # Machine-parseable events (for analysis)
├── prompt.txt           # The exact prompt used
├── summary.json         # Run statistics
├── iter-01.log          # Full output from iteration 1
├── iter-01.tail.txt     # Last 200 lines of iteration 1
├── iter-02.log          # Full output from iteration 2
├── iter-02.tail.txt     # ...
└── analysis/            # Postmortem reports (if enabled)
    ├── spec-compliance.md
    ├── run-quality.md
    └── summary.md
```

### Summary JSON

```json
{
  "run_id": "20250122-143052",
  "start_ms": 1737556252000,
  "end_ms": 1737557891000,
  "total_duration_ms": 1639000,
  "iterations_run": 7,
  "completed_iteration": 7,
  "avg_duration_ms": 234142,
  "last_exit_code": 0,
  "completion_mode": "trailing",
  "model": "opus",
  "exit_reason": "complete_trailing"
}
```

## Postmortem Analysis

When enabled (default), loop runs three analysis passes after completion:

1. **Spec Compliance** - Did the implementation match the spec?
2. **Run Quality** - Any anomalies, protocol violations, or issues?
3. **Summary** - Root cause classification and actionable improvements

Reports are saved to `logs/loop/run-<id>/analysis/`.

Disable with `--no-postmortem` for faster runs during development.

### Manual Analysis

You can also run analysis on any previous run:

```bash
# Analyze the most recent run
loop-analyze

# Analyze a specific run
loop-analyze 20250122-143052

# Analyze an experiment run
loop-analyze 20250122-143052 --experiment

# Actually run the analysis (not just print the prompt)
loop-analyze --run

# Run experiment analysis and write report
loop-analyze 20250122-143052 --experiment --run
```

## Spec and Plan Format

### Spec Structure

Specs should clearly describe **what** to build:

```markdown
# Feature Name

**Status:** Draft | In Progress | Complete
**Last Updated:** YYYY-MM-DD

## Overview
Brief description of the feature.

## Requirements
- Requirement 1
- Requirement 2

## Technical Details
Implementation specifics, schemas, APIs, etc.

## Out of Scope
What this spec explicitly does NOT cover.
```

### Plan Structure

Plans are **checklists** of tasks to complete:

```markdown
# Feature Name - Implementation Plan

## Tasks
- [ ] Task 1 description
- [ ] Task 2 description
- [ ] Task 3 description

## Verification
- [ ] Manual test 1
- [ ] Manual test 2

## Notes
Any context for the implementing agent.

## Blockers Discovered
| Type | Location | Description |
|------|----------|-------------|
| PROD_BUG | file:line | Brief description |
| TEST_INFRA | package/tool | What's missing |
```

Task markers:

- `[ ]` not started
- `[x]` complete
- `[R]` reviewed
- `[~]` blocked/partial (add entry to **Blockers Discovered**)
- `[ ]?` optional/manual QA (doesn't block completion)

## Environment Variables

| Variable | Description |
|----------|-------------|
| `LOOP_CONFIG` | Path to config file (alternative to `--config`) |

## Tips

### Writing Good Specs

- Be specific about data shapes, APIs, and behavior
- Include examples where helpful
- Define edge cases explicitly
- Mark ambiguous areas clearly

### Writing Good Plans

- Keep tasks atomic (one commit each)
- Order by dependency, not preference
- Include verification steps
- Add notes if context is needed

### Debugging Failed Runs

1. Check `logs/loop/run-<id>/iter-NN.log` for the failing iteration
2. Look at `iter-NN.tail.txt` for the last 200 lines
3. Review `analysis/summary.md` if postmortem ran
4. Check if the spec was ambiguous or the plan too vague

### Performance

- Use `--iterations 5` when testing
- Use `--no-postmortem` during development
- Use `--model sonnet` for faster (cheaper) iterations on simpler tasks

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Completed successfully |
| 130 | Interrupted (SIGINT/Ctrl+C) |
| 143 | Terminated (SIGTERM) |
| Other | Claude CLI exit code |

## Related Tools

- [gritty](https://github.com/ohitslaurence/gritty) - AI-powered git commits
- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) - The underlying AI agent
- [gum](https://github.com/charmbracelet/gum) - Terminal UI toolkit

## License

MIT
