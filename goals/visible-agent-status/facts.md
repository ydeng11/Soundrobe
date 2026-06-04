# Visible Agent Status — Facts

- Each assistant reply message in the chat shows an embedded status indicator (icon + label) reflecting the assistant's state: sending, thinking, looking up data, applying changes, completed, or failed.
- Backend trace details (tool_running, tool_result, error events that were previously shown as separate inline system messages) are collapsed into a single expandable section within the assistant's reply message. The default state is collapsed.
- While the assistant is processing, the status indicator updates live — changing from 'sending' → 'thinking' → 'looking up data' → 'applying changes' → 'completed' — and the collapsible details section accumulates entries as they arrive.
- When the assistant encounters an error, the status shows 'failed' with the error message visible in the indicator itself. The collapsible details section contains the full error trace. The user can retry (edit & resend) directly from the failed message.
- The current separate inline system messages for tool_running, tool_result, and action_batch events are replaced by the collapsible details section inside the assistant's reply message. User messages remain unchanged.
- Implementation uses only existing Assistant event streams — no new backend IPC channels are added.
