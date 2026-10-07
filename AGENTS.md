# AGENTS.md

Doctor Whodunit — Safety Evidence Factory (Weeping-Angel repo).
Challenge spec: `README.md` (Definition of Done = acceptance list). Demo: `demo/`
(reference implementation, not authoritative — ADR-003). Idea backlog: `PLAN.md`
(no authority over ADRs, see below).

## Project Memory System

This project maintains institutional knowledge in `docs/project_notes/` for
consistency across sessions and AI tools.

### Memory Files

- **decisions.md** — Architectural Decision Records (ADRs) with context and trade-offs. **Authoritative**: ADRs override `PLAN.md` and the demo where they conflict (ADR-002, ADR-003).
- **key_facts.md** — Project configuration, ports, topics, constants, URLs
- **bugs.md** — Bug log with dates, solutions, and prevention notes
- **issues.md** — Work log with status and context

### Memory-Aware Protocols

**Before proposing architectural changes:**
- Check `docs/project_notes/decisions.md` for existing decisions
- Verify the proposed approach doesn't conflict with past choices
- If it conflicts, acknowledge the existing decision and explain why a change is warranted — an ADR is only overridden by a new ADR, never silently
- `PLAN.md` is an idea backlog, not a spec: it yields to ADRs
- `demo/` is a reference implementation, not authoritative (ADR-003): requirements come from the challenge README + ADRs; demo-derived observations are informational, never project issues

**When encountering errors or bugs:**
- Search `docs/project_notes/bugs.md` for similar issues; apply known solutions if found
- Document new bugs and solutions when resolved

**When looking up project configuration:**
- Check `docs/project_notes/key_facts.md` (ports, uProtocol topics, thresholds, CAN asset layout)
- Prefer documented facts over assumptions

**When completing work:**
- Log completed/open work in `docs/project_notes/issues.md`
- Record binding decisions as new ADRs in `docs/project_notes/decisions.md`

### Style Guidelines

- Bullet lists over tables; concise entries (1–3 lines)
- Always date entries (YYYY-MM-DD); ADRs numbered sequentially (ADR-001, ADR-002, ...)
- Docs and ADRs in English; manual cleanup of stale entries
