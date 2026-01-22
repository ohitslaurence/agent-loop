# loop

Run Claude Code in a loop to implement spec-driven tasks autonomously.

## Overview

`loop` executes Claude Code repeatedly, feeding it a spec and plan file. The agent works through plan items one at a time, committing after each, until all tasks are complete.

The loop stops when Claude outputs `<promise>COMPLETE</promise>`.

## Installation

```bash
git clone git@github.com:ohitslaurence/agent-loop.git ~/dev/personal/agent-loop
cd ~/dev/personal/agent-loop
./install.sh
```

This creates symlinks in `~/.local/bin/`. Use `./install.sh --global` for `/usr/local/bin/`.

### Dependencies

**Required:**
- [Claude Code CLI](https://docs.anthropic.com/en/docs/claude-code) (`claude`)
- [gritty](https://github.com/ohitslaurence/gritty) for AI commits (referenced in default prompt)

**Optional:**
- [gum](https://github.com/charmbracelet/gum) for interactive spec picker and styled output

## Quick Start

```bash
# Initialize config in your project
cd your-project
loop --init-config

# Create a spec and plan
mkdir -p specs specs/planning
echo "# My Feature" > specs/my-feature.md
echo "- [ ] Task 1" > specs/planning/my-feature-plan.md

# Run the loop
loop specs/my-feature.md
```

## Usage

```
loop [spec-path] [plan-path] [options]

Arguments:
  spec-path           Path to spec file (optional if gum available)
  plan-path           Path to plan file (defaults to <plans_dir>/<spec>-plan.md)

Options:
  --iterations <n>    Maximum loop iterations (default: 50)
  --log-dir <path>    Base log directory (default: logs/loop)
  --model <name>      Claude model or alias (default: opus)
  --completion-mode   Completion detection (exact|trailing, default: trailing)
  --prompt <path>     Custom prompt file
  --no-postmortem     Disable automatic post-run analysis
  --no-gum            Disable gum UI, use plain output
  --summary-json      Write summary JSON at end of run (default: enabled)
  --no-wait           Skip completion screen wait
  --config <path>     Load config file (overrides project config)
  --init-config       Create a project config file and exit
```

## Project Configuration

Run `loop --init-config` to create `.loop/config`:

```ini
specs_dir="specs"
plans_dir="specs/planning"
log_dir="logs/loop"
model="opus"
iterations=50
completion_mode="trailing"
postmortem=true
summary_json=true
no_wait=false
no_gum=false
# prompt_file=""
# context_files="specs/README.md specs/planning/SPEC_AUTHORING.md"
```

### Context Files

Use `context_files` to include additional files as `@path` references in the prompt:

```ini
context_files="specs/README.md specs/planning/SPEC_AUTHORING.md CLAUDE.md"
```

This generates a prompt starting with:
```
@specs/my-feature.md @specs/planning/my-feature-plan.md @specs/README.md @specs/planning/SPEC_AUTHORING.md @CLAUDE.md
```

## Custom Prompt

Create `.loop/prompt.txt` in your project to override the default prompt.

Use `SPEC_PATH` and `PLAN_PATH` as placeholders - they'll be substituted at runtime.

Example:
```
@SPEC_PATH @PLAN_PATH @specs/README.md

You are an implementation agent. Read the spec and plan, then:
1. Pick ONE unchecked task
2. Implement it
3. Mark it [x] in the plan
4. Commit with gritty commit --accept
5. If all done: output <promise>COMPLETE</promise>
```

## Logs and Reports

Each run creates a directory under `logs/loop/run-<timestamp>/`:

- `run.log` - Human-readable log
- `report.tsv` - Machine-parseable event log
- `prompt.txt` - The prompt used
- `summary.json` - Run summary
- `iter-NN.log` - Full output per iteration
- `iter-NN.tail.txt` - Last 200 lines per iteration
- `analysis/` - Postmortem reports (if enabled)

## Completion Protocol

The loop detects completion when Claude outputs:
```
<promise>COMPLETE</promise>
```

**Modes:**
- `exact`: Entire response must be exactly `<promise>COMPLETE</promise>`
- `trailing` (default): Token must be the last non-empty line

## Postmortem Analysis

When enabled (default), runs three analysis passes after completion:
1. Spec compliance check
2. Run quality analysis
3. Summary synthesis

Disable with `--no-postmortem`.

## Environment Variables

- `LOOP_CONFIG` - Path to config file (alternative to `--config`)

## Tips

- Start with small, well-defined specs
- Keep plan items atomic (one commit each)
- Use `--iterations 5` for testing
- Review logs when stuck
- Custom prompts can reference project-specific docs with `@path` syntax
