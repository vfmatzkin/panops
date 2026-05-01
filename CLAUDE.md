@AGENTS.md

## Claude Code addenda

- For non-trivial architectural changes, use plan mode (the `Plan` agent or `EnterPlanMode`) before touching code. Trivial fixes (typos, single-line changes, obvious renames) can skip planning.
- Use the `superpowers:brainstorming` skill before opening a slice spec, and `superpowers:writing-plans` before opening a slice plan. **NEVER** invoke other implementation skills mid-brainstorm — brainstorming's terminal state is `writing-plans`.
- Use the `superpowers:subagent-driven-development` skill to execute slice plans, dispatching one fresh implementer subagent per task with two-stage review (spec compliance → code quality) between tasks.
- Personal preferences for this machine go in `CLAUDE.local.md` (gitignored).
