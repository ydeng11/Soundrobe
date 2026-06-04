# Fix: Agent Loop Degradation & Context Waste

## Problem

Session #1780539959122-506955 revealed three interacting failure modes:

1. **NetEase encrypted keys waste ~8K/tool-call** — The `description` field in every track contains a ~400-char NetEase Cloud Music encrypted token (`163 key(Don't modify):...`). When inspecting 20 tracks, that is ~8,000 chars of useless data injected into the system prompt every loop. Over 10 loops it accumulated ~80K of garbage tokens.

2. **Agent repeats the same tool call without adapting** — The agent called `tracks.inspect({})` 6 consecutive times, getting the same surprising result (1 track instead of 53), but never tried passing explicit paths or using a different approach. The runtime provides no feedback that the agent is looping.

3. **No selection-context anchor in tool results** — The agent had no way to detect the user's selection changed between calls. `tracks.inspect({})` returned different results at different points, but the tool output didn't surface the selection change explicitly.

## Changes

### 1. Truncate long description/comment fields in `tracks.inspect` output

**File:** `frontend/electron/handlers/assistant.ts` — `tracks.inspect` executor

**Current:**
```
Comment: (none) | Description: 163 key(Don't modify):L64FU3W4YxX3ZFTmbZ+8/YRNBwsv/sCAk...
```
Full encrypted token shown. ~400 chars per track × 20 tracks = ~8K wasted per inspection.

**Change:** Truncate `description` and `comment` fields to 60 chars in the tool output. Append `...` if truncated.

**Why:** The encrypted key is never useful for the LLM agent. The field holds a NetEase DRM token, not artist/album metadata. This is the single biggest context-waster in the tool output.

**Test:** Update the `tracks.inspect` test assertion to match the truncated format.

### 2. Surface selection context in `tracks.inspect` output header

**File:** `frontend/electron/handlers/assistant.ts` — `tracks.inspect` executor

**Current:**
```
Inspecting 20 track(s) (showing first 20 of 53):
```

**Change:** Add selection scope to the first line:
```
Inspecting 20 track(s) (showing first 20 of 53) — 1 selected from 53 library tracks:
```

**Why:** This lets the agent immediately detect when the user changed their selection between calls. If it sees "1 selected" while trying to inspect all tracks, it understands the scope narrowed and can adapt (e.g., pass explicit paths).

### 3. Add repeated-tool-call circuit breaker in `AssistantRuntime`

**File:** `frontend/electron/services/AssistantRuntime.ts` — `send()` method

**Change:** Before each API call in the tool loop, check if the agent made the same tool call (toolName + normalized args) 3+ consecutive times. When detected, inject a warning into the conversation history.

**Signature check logic:**
```typescript
const recentToolCalls = this.conversation
  .filter(m => m.role === "assistant")
  .map(m => {
    try { return JSON.parse(m.content); } catch { return null; }
  })
  .filter(p => p?.type === "tool_call")
  .slice(-5);

// Check for repeats
const callSignatures = recentToolCalls.map(t => `${t.toolName}|${JSON.stringify(sortKeys(t.args))}`);
const lastSig = callSignatures.at(-1);
const repeatCount = callSignatures.filter(s => s === lastSig).length;

if (repeatCount >= 3) {
  this.conversation.push({
    role: "system",
    content: `[System note: You called ${lastSig!.split("|")[0]} with the same arguments ${repeatCount} times. ` +
      `Consider a different approach or different arguments.]`
  });
}
```

**Why:** This is the exact pattern seen in session #506955 — calling `tracks.inspect({})` 6 times. A system interrupt breaks the loop and tells the agent to try something else. The hint costs ~200 tokens but prevents spending thousands on more loop iterations.

### 4. Enhance max-steps diagnostic with repeated-call info

**File:** `frontend/electron/services/AssistantRuntime.ts` — `send()` max-steps handler

**Current:** Shows last 4 conversation entries (role + first 200 chars).

**Change:** If repeated calls were detected, append a suggestion:
```
... The assistant called "${toolName}" with the same arguments ${repeatCount} times.
This suggests the tool results were not what was expected. Try rephrasing your
request or providing more specific file paths.
```

**Why:** The user gets a concrete hint about what went wrong instead of a generic "try rephrasing" message.

## Success Criteria

1. `tracks.inspect` output for a track with a long description shows truncated text
2. `tracks.inspect` header shows selection count + library total
3. When calling the same tool with same args 3+ times, a system hint appears in conversation
4. Max-steps message includes repeated-call diagnostic (when applicable)
5. All existing tests pass unchanged

## Verification

```bash
cd frontend && npm test               # Unit tests
cd frontend && npx vitest run test/components/AssistantPanel.test.ts  # Component tests
cd frontend && npx vitest run test/services/   # Runtime & service tests
```

## File Summary

| File | Changes |
|------|---------|
| `frontend/electron/handlers/assistant.ts` | Truncate description/comment to 60 chars; add selection context header |
| `frontend/electron/services/AssistantRuntime.ts` | Repeated-call detection + system hint injection; enhanced max-steps diagnostic |
| `frontend/test/handlers/assistant.test.ts` | Update test assertions for truncated description/comment format |
