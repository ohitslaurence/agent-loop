# Roadmap

Long-term vision for agent-loop. See `TODO.md` for near-term hardening.

---

## Phase 1: Foundation for Visibility

### Dashboard MVP
- [ ] Basic React app scaffolding
- [ ] Poll daemon HTTP endpoint for current state
- [ ] Display active projects, current agent, iteration count, branch status
- [ ] Show real-time logs/output stream (WebSocket or SSE from Rust)

### Notifications v1
- [ ] Slack webhook integration on task complete/fail
- [ ] Include: project name, success/fail, iteration count, branch name
- [ ] Optional: desktop notifications for local dev

---

## Phase 2: Agent Specialization

### Specialist System
- [ ] Agent config directory with system prompts (.md or .toml files)
- [ ] Define specializations: frontend, backend, reviewer, devops, tests
- [ ] Task router - LLM call or keyword matching to pick specialist
- [ ] Update dashboard to show which specialist is active

### Adversarial Review
- [ ] After "coder" agent completes, run "reviewer" agent as a gate
- [ ] Reviewer can request changes (loops back) or approve (proceeds to merge)
- [ ] Track review iterations separately in dashboard

---

## Phase 3: Smarter Orchestration

### Stuck Detection
- [ ] Track iteration count, flag if exceeds threshold
- [ ] Detect circular diffs (agent keeps making/reverting same change)
- [ ] Escalation options: try different specialist, pause for human, abandon

> Note: Basic consecutive failure detection is tracked in `TODO.md` P1.

### Task Decomposition
- [ ] Optional "planner" agent that breaks spec into subtasks
- [ ] Dependency graph between subtasks
- [ ] Parallel execution of independent subtasks

---

## Phase 4: Polish & Ops

### Dashboard v2
- [ ] Historical view - completed tasks, time taken, iterations needed
- [ ] Cost tracking (tokens used per task)
- [ ] Filter/search across projects
- [ ] Dark mode
- [ ] Settings page - view/edit agent prompts (implementation, review, postmortem)
- [ ] Expose prompts via daemon API endpoint (GET /config/prompts)

### Notifications v2
- [ ] "Needs attention" alerts (stuck, review requested, merge conflict)
- [ ] Configurable channels per project or severity
- [ ] Daily digest option

### Reliability
- [ ] Checkpointing for daemon restart recovery
- [ ] Human-in-the-loop flag for high-risk merges
- [ ] Audit log of all agent actions

---

## Bonus Ideas

- **Agent memory/context** - Agents can read summaries of what other agents did on related tasks
- **Confidence scoring** - Agent self-reports confidence; low confidence triggers review
- **A/B approaches** - Spin up two agents with different approaches, pick the one that passes tests
- **Spec validation agent** - Before starting, an agent reviews the spec for clarity and asks clarifying questions
- **Review-to-agent handoff** - From dashboard review view, add comments/feedback, then dispatch an agent to address them (like PR review → fix cycle)
