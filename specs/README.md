# Platform Specifications

This index maps durable system specs to their implementation plans and code locations.
Keep this file current whenever a new spec or plan is added.

## Core Specs

| Spec | Plan | Code | Purpose |
| ---- | ---- | ---- | ------- |

## Architecture Specs

| Spec                                             | Plan                                                                | Code   | Purpose                                              |
| ------------------------------------------------ | ------------------------------------------------------------------- | ------ | ---------------------------------------------------- |
| [orchestrator-daemon.md](./orchestrator-daemon.md) | [orchestrator-daemon-plan.md](./planning/orchestrator-daemon-plan.md) | `crates/loopd`, `crates/loopctl`, `crates/loop-core` | Rust daemon + CLI orchestrator for agent loop |
| [orchestrator-daemon-extended.md](./orchestrator-daemon-extended.md) | [orchestrator-daemon-extended-plan.md](./planning/orchestrator-daemon-extended-plan.md) | `crates/loopd`, `crates/loopctl` | Runner pipeline + local scaling + readiness probe |
| [worktrunk-integration.md](./worktrunk-integration.md) | [worktrunk-integration-plan.md](./planning/worktrunk-integration-plan.md) | `crates/loopd/src/worktree.rs` | Worktrunk-backed worktree lifecycle and provider selection |
| [distributed-scheduling.md](./distributed-scheduling.md) | [distributed-scheduling-plan.md](./planning/distributed-scheduling-plan.md) | `crates/loopd-controller`, `crates/loopd-worker` | Deferred: multi-host controller/worker scheduling |
| [postmortem-analysis.md](./postmortem-analysis.md) | [postmortem-analysis-plan.md](./planning/postmortem-analysis-plan.md) | `crates/loopd`, `crates/loopctl` | Run postmortem analysis + summary.json artifacts |

## Research Notes

| Spec | Plan | Code | Purpose |
| ---- | ---- | ---- | ------- |

## Planning Conventions

- Plans live in `specs/planning/` and should be linked here once created.
- Specs live in `specs/` and remain stable; plans evolve as work is completed.
