# Upgrade Electron to Latest Stable

Upgrade Electron from 34.5.8 to 42.3.3 through incremental stops at 37.x and 40.x, verifying at each stop. Related dependencies are upgraded only when they block compilation or tests. The ABI mismatch with system Node.js (26, ABI 147) remains unaddressed — the existing inline rebuild fix handles it.

## Shared Understanding

See [facts.md](./facts.md) for verified facts.

## Execution Plan

See [plan.md](./plan.md) for ordered steps.

## Done Condition

1. `package.json` has `"electron": "^42.3.3"`
2. `npm run typecheck` exits 0
3. `npm test` passes all 700+ tests
4. `npm run dev` launches the app without errors
5. Each intermediate version (37, 40, 42) is committed separately
