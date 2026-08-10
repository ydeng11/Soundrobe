# Plan Executor — Multi-Step Agent Pipeline

## Context

The current assistant flow is single-shot: the LLM calls one tool, gets one result, and stops. For a chain like *"change AlbumArtist to 法老 and fix Artist from the file name. Also create Artists based on fixed Artist tag"*, the LLM must inspect files, reason about filenames, then construct an `edit_metadata` call — all in one implicit step. There's no structured plan, no dependency ordering, no inter-step state, and no validation.

We need a Plan-and-Solve layer that sits on top of the existing tools.

### Why Plan-and-Solve and not ReAct or Tree-of-Thoughts?

**Tree of Thoughts** forks multiple execution paths and backtracks — overkill for audio tagging. The domain is narrow (inspect → decide → edit → verify), always 3–5 steps, and doesn't need competing branches.

**ReAct** alternates *thought → action → observation* for every step, requiring an LLM round-trip per step. For a deterministic chain like the 法老 scenario, the steps are known upfront. Every intermediate "what should I do next?" call burns tokens and latency for zero benefit.

**Plan-and-Solve** fits because: (a) the agent plans upfront and the executor runs without intermediate LLM calls, (b) the plan is short and deterministic, (c) it matches the preview/approve model — one plan produces N preview batches, the user approves once.

## Approach

A `PlanExecutor` service that:

1. Takes a `Plan` (array of `PlanStepDef` with dependencies)
2. Resolves execution order via topological sort
3. Resolves `$stepId.field` argument references from previous step outputs
4. Executes each step through the existing `AssistantToolRegistry`
5. Collects preview batches for user approval
6. Runs optional post-step validation

The LLM defines plans via a new `create_plan` assistant tool. The executor runs the plan and presents all batches for one-shot user approval.

## Files to create/modify

| File | Action |
|------|--------|
| `electron/services/PlanExecutor.ts` | **Create** — the executor class |
| `electron/handlers/assistant.ts` | **Modify** — register `create_plan` tool, wire PlanExecutor |

## Reuse

- `AssistantToolRegistry` — already has `execute(toolName, args)` → `AssistantToolResult` ✅
- `AssistantRuntime` — already has `createActionBatch()` and `getActionBatch()` ✅
- All existing tools (`tracks.inspect`, `edit_metadata`, etc.) work unchanged under the plan ✅

## Design

### Types (PlanExecutor.ts)

```typescript
interface PlanStepDef {
  id: string;                    // "step-1"
  label?: string;                // "Inspect filenames"
  tool: string;                  // "tracks.inspect"
  args: Record<string, unknown>; // { paths: "$step-1.tracks" }
  depends_on?: string[];         // ["step-0"]
  validate?: (output) => { pass: boolean; errors: string[] };
}

interface Plan { steps: PlanStepDef[]; }

interface PlanResult {
  stepOutputs: PlanStepOutput[];
  batches: AssistantActionBatch[];
  errors: PlanStepError[];
  scratchpad: Map<string, unknown>;
}
```

### Executor flow per step

```
resolveArgs($refs) → registry.execute(tool, args) → store in scratchpad
→ collect batch → run validate → next step
```

### Argument reference syntax

- `"$step-1"` — entire output of step-1
- `"$step-1.tracks"` — `.tracks` property of step-1's output data
- `["$step-1.path", "/static/path"]` — works inside arrays too

### Dependency resolution

Topological sort with cycle detection. If step B depends on step A, A runs first.

### LLM interface

A new assistant tool `create_plan`:

```typescript
{
  name: "create_plan",
  description: "Define a multi-step execution plan. Steps run in dependency order. "
    + "Use $stepId.field to reference earlier step outputs in arguments.",
  inputSchema: {
    properties: {
      plan_description: { type: "string", description: "Explain the overall goal" },
      steps: {
        type: "array",
        items: {
          properties: {
            id: { type: "string" },
            label: { type: "string" },
            tool: { type: "string" },
            args: { type: "object" },
            depends_on: { type: "array", items: { type: "string" } },
          },
          required: ["id", "tool"],
        },
      },
    },
    required: ["steps"],
  },
}
```

## Example: "change AlbumArtist to 法老 and fix Artist from filename"

The LLM constructs this plan:

```json
{
  "steps": [
    {
      "id": "inspect",
      "label": "Read current track metadata",
      "tool": "tracks.inspect",
      "args": { "paths": [] }
    },
    {
      "id": "extract",
      "label": "Extract artist from filename",
      "tool": "tracks.inspect",
      "args": { "paths": "$inspect.paths" },
      "depends_on": ["inspect"]
    },
    {
      "id": "edit",
      "label": "Set albumArtist, artist, artists",
      "tool": "edit_metadata",
      "args": {
        "target_scope": "active_album",
        "standard_updates": {
          "albumArtist": "法老",
          "artist": "法老",
          "artists": ["法老"]
        }
      },
      "depends_on": ["extract"]
    }
  ]
}
```

The executor:
1. Runs `tracks.inspect` → stores filenames in scratchpad under `"inspect"`
2. Resolves `$inspect.paths` → passes to second `tracks.inspect` → stores metadata
3. Resolves any refs in `edit_metadata` → runs it → creates preview batch
4. Returns all batches for one-shot user approval

## Steps

- [ ] Create `electron/services/PlanExecutor.ts` with types, topological sort, arg resolution, step executor
- [ ] Register `create_plan` tool in `assistant.ts` that calls `PlanExecutor.execute()`
- [ ] Wire PlanExecutor to the assistant runtime so all batches return for approval
- [ ] Add test for dependency resolution (linear, diamond, cycle detection)
- [ ] Add test for `$ref` argument resolution
- [ ] Add end-to-end test: "法老 scenario" as a 3-step plan

## Verification

```bash
npx vitest run test/services/PlanExecutor.test.ts --reporter=verbose
# All existing tool tests must still pass
npx vitest run test/handlers/auto-tag.test.ts test/services/TrackTagService.test.ts
```
