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
    <main className="flex min-h-screen items-center justify-center bg-[#f5f0e8] px-4 text-[#28231d]">
      <form
        onSubmit={handleSubmit}
        className="flex w-full max-w-sm flex-col gap-5 rounded-2xl border border-[#ded5c8] bg-[#fffdf9] p-7 shadow-[0_18px_50px_rgba(65,48,32,0.08)]"
      >
        <div>
          <p className="text-xs font-semibold uppercase tracking-[0.18em] text-[#a85732]">Soundrobe service</p>
          <h1 className="mt-2 font-serif text-2xl font-semibold">Sign in to Soundrobe</h1>
          <p className="mt-2 text-sm text-[#71675d]">Use the shared operator password.</p>
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
            className="rounded-xl border border-[#d9cdbd] bg-[#faf7f1] px-3 py-2 font-normal outline-none focus:border-[#a85732] focus:ring-2 focus:ring-[#a85732]/20"
          />
        </label>
        {error && <p className="text-sm text-red-600" role="alert">{error}</p>}
        <button
          type="submit"
          disabled={submitting}
          className="rounded-full bg-[#332c25] px-3 py-2.5 text-sm font-semibold text-[#fffaf2] hover:bg-[#4a4037] disabled:opacity-60"
        >
          {submitting ? "Signing in…" : "Sign in"}
        </button>
      </form>
    </main>
  );
}
