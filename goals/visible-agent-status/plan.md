# Plan: Visible Agent Status — Assistant Panel Enrichment

## Context

The Assistant chat panel currently shows backend trace events (`tool_running`, `tool_result`, `error`, `action_batch_*`) as separate inline system messages mixed into the chat stream. This is noisy and the user has no quick way to see "what's the assistant doing right now" or "did it fail."

We replace the separate system messages with a **per-message status indicator** and a **collapsible details section** embedded in each assistant reply — giving the user a clean view with drill-down when needed.

## Approach

1. **Extend `ChatMessage` type** to carry a `status` field (enum of states) and a `details` field (array of backend trace entries).
2. **Modify event handlers** in `AssistantPanel.tsx` to accumulate events into a single pending assistant message, rather than creating separate system messages.
3. **Update `MessageBubble`** to render:
   - An embedded status indicator (icon + label, live-updating)
   - The main message content
   - A collapsible details section for the backend trace
4. **Remove** the separate `tool_running`, `tool_result`, `action_batch_*` system message rendering.
5. **Error state**: `failed` status with visible error; retry via edit-and-resend.
6. **Write tests**.

## State Machine (per assistant reply)

```
sending → thinking → looking up data → applying changes → completed
                                                          → failed
```

Transitions are driven by existing `AssistantEvent.type` values.

## Files to Modify

| File | Changes |
|------|---------|
| `frontend/src/components/AssistantPanel.tsx` | All UI changes — message type, event handlers, MessageBubble rendering |
| (no backend changes) | Constraint: use existing event streams only |

## Event → Status & Details Mapping

| AssistantEvent.type | Status transition | Details entry |
|---------------------|-------------------|---------------|
| `tool_running` | → `thinking` or → `looking up data` | "🔧 {message}" |
| `tool_result` | (no state change) | "📋 {message}" |
| `error` | → `failed` | "⚠️ {message}" |
| `action_batch_created` | → `applying changes` | (summary) |
| `action_batch_applied` | → `completed` | "✅ {message}" |
| `action_batch_failed` | → `failed` | "⚠️ {message}" |
| `message` | → `completed` (final) | — |
| `completed` | → `completed` | — |
| `cancelled` | → `failed` | "⏹️ Cancelled" |

## Steps

### Step 1: Extend `ChatMessage` type

Add `status` and `details` fields to `ChatMessage`. The status type is an enum of the 6 states. The details type is an array of `{icon, text, timestamp?}` entries.

**Verification:** TypeScript compiles without errors.

### Step 2: Rewrite event handler to accumulate into pending message

Replace the current event handler that creates separate `ChatMessage` entries for each event type. Instead:
- On first event (after user sends a message): create a pending assistant message with `status: "sending"` and empty details
- On each subsequent event: update the pending message's status and append to details
- On `message` event: set the assistant's reply text and mark as completed
- On `error`/`cancelled`: set status to `failed`, keep accumulated details

**Verification:** Open assistant, send a prompt, observe no more separate yellow/green system messages — instead a single assistant message with live status.

### Step 3: Update `MessageBubble` rendering

Render the status indicator:
- A compact row at the top of the message bubble: icon + colored label
- The main message content (the assistant's reply text, or "…" while pending)
- A collapsible details section: click to expand/collapse; shows accumulated trace entries

**Verification:** Status visual matches states, collapsible works, layout doesn't break.

### Step 4: Remove separate system message rendering

Delete the code paths that produce `ChatMessage` entries with `type: "tool_running" | "tool_result"`. The `action_batch_created`/`applied`/`rejected`/`failed` events are also absorbed into the status/details model.

**Verification:** No yellow or green system messages appear in the chat.

### Step 5: Error state with retry

When status is `failed`, show the error prominently in the status row. The existing edit-and-resend mechanism (pencil icon on user messages) already exists — ensure the user can click edit on their original prompt and resend.

**Verification:** Send a prompt that triggers an error, confirm status shows "failed" with error, confirm edit-and-resend works.

### Step 6: Write tests

Tests should cover:
- Pending message accumulation from sequential events
- Status transitions for each event type
- Collapsible details expand/collapse
- Error state rendering
- That no separate system messages are created

## Verification

1. `just fe-typecheck` — no type errors
2. `just fe-test` — new + existing tests pass
3. Manual: open Assistant, send prompt, observe:
   - Status indicator cycles through states
   - Backend trace is collapsed by default
   - Click to expand shows trace entries
   - On error: failed status + retry works
