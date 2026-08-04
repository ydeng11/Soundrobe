# Advisor prompt patterns

Select one primary checklist. Add items from another only when the problem genuinely crosses task types.

## Stage directives

### Initial

- Judge whether exploration is headed in the right direction.
- Give the shortest viable approach, the main risk, and two or three first steps.

### Recovery

- Explain what failed or became confused using supplied evidence.
- State what to stop doing and give the corrected path.

### Final-check

- Reconcile the result against every original requirement and constraint.
- Judge whether verification is sufficient and identify missing edge cases.
- Give explicit sign-off or use `Not done yet` and list what remains.

## Planning

- Test whether the plan covers the stated goal and success criteria.
- Find missing dependencies, ordering constraints, migration needs, and rollback points.
- Identify assumptions that require evidence before implementation.
- Recommend the smallest verifiable sequence of work.

## Review

- Inspect the supplied diff when present; otherwise inspect the named artifacts and immediate call paths for correctness and regressions.
- Prioritize findings by impact and cite exact files or lines when available.
- Check error handling, security boundaries, compatibility, and test intent.
- Avoid style-only findings unless they obscure a real defect.

## Coding

- Recommend the smallest implementation that matches existing conventions.
- Name the code paths, edge cases, and tests that should change.
- Provide pseudocode or a focused patch sketch only when it makes the advice clearer.
- Do not edit the workspace or broaden the requested feature.

## Debugging

- Rank hypotheses by fit with observed evidence.
- Separate the root cause from symptoms and incidental failures.
- Propose discriminating, read-only checks before any fix.
- State what evidence would falsify the leading hypothesis.

## Architecture

- Compare viable options against explicit constraints.
- Address operational cost, failure modes, reversibility, and migration.
- Prefer the simplest option that meets current requirements.
- State which future condition would justify a more complex design.

## Research

- Separate sourced facts, inferences, and unknowns.
- Prefer primary or authoritative evidence.
- Call out stale, conflicting, or weak evidence.
- Tie findings back to the concrete decision.

## Testing

- Check that tests encode intent and fail when the protected behavior changes.
- Cover boundaries, negative paths, integration seams, and realistic fixtures.
- Identify false-positive and false-negative risks.
- Recommend the narrowest reliable verification commands.

## Required response

Use this compact structure:

```text
Verdict: On track | Course-correct | Not done yet

1. <next action with evidence reference>
2. <next action with evidence reference>
... no more than five actions

Risks or unknowns: <only material unresolved points>
```

If evidence is insufficient, make action 1 the exact read, command, or observation needed. Do not invent evidence or silently resolve conflicts.
