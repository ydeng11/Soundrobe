import React, { useState } from "react";

interface WebLoginScreenProps {
  onLogin: (password: string) => Promise<void>;
}

export function WebLoginScreen({ onLogin }: WebLoginScreenProps) {
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const handleSubmit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setSubmitting(true);
    setError(null);
    try {
      await onLogin(password);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Sign in failed");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <main className="flex min-h-screen items-center justify-center bg-surface px-4 text-text-primary">
      <form
        onSubmit={handleSubmit}
        className="flex w-full max-w-sm flex-col gap-4 rounded-xl border border-border bg-white p-6 shadow-sm"
      >
        <div>
          <h1 className="text-lg font-semibold">Sign in to Soundrobe</h1>
          <p className="mt-1 text-sm text-text-muted">Use the shared operator password.</p>
        </div>
        <label className="flex flex-col gap-1 text-sm font-medium" htmlFor="web-password">
          Password
          <input
            id="web-password"
            type="password"
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            autoComplete="current-password"
            required
            className="rounded-md border border-border px-3 py-2 font-normal outline-none focus:border-accent"
          />
        </label>
        {error && <p className="text-sm text-red-600" role="alert">{error}</p>}
        <button
          type="submit"
          disabled={submitting}
          className="rounded-md bg-accent px-3 py-2 text-sm font-medium text-white disabled:opacity-60"
        >
          {submitting ? "Signing in…" : "Sign in"}
        </button>
      </form>
    </main>
  );
}
