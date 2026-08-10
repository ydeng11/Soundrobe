# Goal: Visible Agent Status — Assistant Panel Enrichment

Enrich the Assistant chat panel so users can see, at a glance, what the assistant is doing (sending, thinking, looking up data, applying changes, completed, failed) — with backend trace details collapsed by default but expandable on demand.

## Facts

See [facts.md](./facts.md) for the 6 accepted facts defining the feature.

## Plan

See [plan.md](./plan.md) for the execution plan.

## Done Condition

- Each assistant reply message shows an embedded live-updating status indicator with a collapsible details section for the backend trace
- No separate inline system messages (`tool_running`, `tool_result`, `action_batch_*`) appear in the chat stream
- Error state shows `failed` status with error message and supports edit-and-resend retry
- All existing tests pass and new tests cover the accumulation and rendering logic
- TypeScript compiles without errors
