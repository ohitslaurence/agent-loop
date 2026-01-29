# Distributed Scheduling and Worker Pool

**Status:** Draft
**Version:** 1.0
**Last Updated:** 2026-01-29

---

## 1. Overview
### Purpose
Enable multi-host execution by separating the control plane (scheduler + API) from workers that run loop steps. The system should allow multiple machines to execute runs in parallel with durable state, lease-based safety, and centralized observability.

### Goals
- Scale loop execution across multiple hosts while preserving current loop semantics.
- Keep a single controller as the source of truth for run/step state.
- Use lease-based step assignment to tolerate worker crashes.
- Support shared filesystem and git-clone workspace strategies.
- Keep local HTTP + SSE control plane for UI compatibility.

### Non-Goals
- Cross-region replication or active-active controllers.
- Auto-scaling infrastructure or Kubernetes deployment.
- Full UI; only API compatibility is required.

---

## 2. Architecture
### Components
- **Controller**: owns scheduling, persistence, leases, and API/SSE.
- **Worker**: executes steps, streams output, uploads artifacts.
- **Storage**: Postgres for distributed mode; SQLite remains for single-host mode.
- **Artifact Store**: local filesystem (single-host) or object store (distributed).
- **Workspace Resolver**: prepares a workspace using shared filesystem or git clone.

### Dependencies
- Postgres (distributed mode).
- Object storage (S3/MinIO) for artifact mirroring when workers are remote.
- Git CLI for clone-based workspace strategy.

### Module/Folder Layout
```
crates/
  loop-core/           # shared types/config/events
  loopd-controller/    # HTTP API, scheduler, leases
  loopd-worker/        # step execution, artifacts, heartbeats
  loopctl/             # CLI
```

---

## 3. Data Model
### Core Types
- Worker: id, hostname, status, last_heartbeat, capabilities.
- Lease: step_id, worker_id, expires_at, attempt.
- WorkspaceSource: shared_fs | git.
- ArtifactStore: local | s3.

### Storage Schema
Additions for distributed mode:

| Table | Key Columns | Notes |
| --- | --- | --- |
| workers | id TEXT PK, hostname TEXT, status TEXT, last_heartbeat INTEGER, capabilities_json TEXT | Registered workers |
| leases | step_id TEXT PK, worker_id TEXT, expires_at INTEGER, attempt INTEGER | Lease ownership |
| runs | add workspace_source TEXT, repo_url TEXT, repo_ref TEXT, artifact_store TEXT | Remote workspace metadata |

---

## 4. Interfaces
### Public APIs
Controller HTTP (localhost or internal network):
- `POST /workers/register` {hostname, capabilities}
- `POST /workers/{id}/heartbeat` {status, metrics}
- `POST /workers/{id}/claim` -> {step_id, run_id, phase, lease_expires_at}
- `POST /steps/{id}/complete` {status, exit_code, output_path, artifact_refs}
- `POST /steps/{id}/events` {event_type, payload}

### Internal APIs
- `storage::acquire_lease(step_id, worker_id, ttl)`
- `storage::renew_lease(step_id, worker_id, ttl)`
- `storage::release_lease(step_id, worker_id)`
- `artifact_store::put(path, bytes) -> uri`
- `workspace::prepare(run) -> workspace_path`

### Events (names + payloads)
- `WORKER_REGISTERED`: {worker_id, hostname}
- `WORKER_HEARTBEAT`: {worker_id, status, metrics}
- `LEASE_GRANTED`: {step_id, worker_id, expires_at}
- `LEASE_EXPIRED`: {step_id, worker_id}

---

## 5. Workflows
### Main Flow
```
worker register -> claim step -> execute -> stream output -> complete step
controller -> verify -> watchdog -> enqueue next step
```

### Workspace Strategies
- **shared_fs**: controller provides `workspace_root`; worker uses it directly.
- **git**: worker clones repo_url at repo_ref, then uses worktree rules from `crates/loopd/src/git.rs`.

### Lease Flow
1. Worker claims a step; controller grants lease with TTL.
2. Worker renews lease on heartbeat.
3. If lease expires, controller requeues the step and emits `LEASE_EXPIRED`.

### Retry/Backoff
- Step failures follow existing retry/backoff rules.
- Lease expiration increments attempt count and requeues.

---

## 6. Error Handling
### Error Types
- Worker unreachable or heartbeat timeout.
- Lease conflict or stale completion.
- Artifact upload failure.
- Workspace preparation failure (clone or shared path).

### Recovery Strategy
- Missed heartbeats mark worker offline and requeue leased steps.
- Stale completion is rejected; worker instructed to stop.
- Artifact upload failures are retried; if exhausted, mark step FAILED.

---

## 7. Observability
### Logs
- Controller logs include worker_id and lease_id for all step actions.
- Workers emit structured step logs and upload artifact references.

### Metrics
- workers_online, lease_expired_count, step_retry_count.

### Traces
- Optional in v0.1; reserved for future.

---

## 8. Security and Privacy
### AuthZ/AuthN
- Worker requests authenticated via token header.
- Controller binds to localhost by default; configurable for private networks.

### Data Handling
- Artifacts stored in object store or shared filesystem; paths stored in DB.
- Do not transmit repo secrets to workers by default.

---

## 9. Migration or Rollout
### Compatibility Notes
- Single-host mode continues to use SQLite + local artifacts.
- Distributed mode switches to Postgres + artifact store.

### Rollout Plan
1. Add storage abstraction for SQLite/Postgres.
2. Introduce controller and worker binaries.
3. Gate distributed mode behind config flags.

---

## 10. Open Questions
- Should workers be allowed to run with shared credentials, or per-worker tokens only?
- What is the minimum artifact store for v0.1 (S3-only vs local NFS)?
