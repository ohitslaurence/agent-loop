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

## Research Notes

| Spec | Plan | Code | Purpose |
| ---- | ---- | ---- | ------- |

## Planning Conventions

- Plans live in `specs/planning/` and should be linked here once created.
- Specs live in `specs/` and remain stable; plans evolve as work is completed.
