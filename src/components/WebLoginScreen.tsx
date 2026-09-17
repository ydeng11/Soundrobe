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
    <main className="web-login">
      <form
        onSubmit={handleSubmit}
        className="web-login-form"
      >
        <div>
          <p className="web-eyebrow">Soundrobe service</p>
          <h1 className="web-page-title">Sign in to Soundrobe</h1>
          <p className="web-page-description">Use the shared operator password.</p>
        </div>
        <label className="web-login-label" htmlFor="web-password">
          Password
          <input
            id="web-password"
            type="password"
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            autoComplete="current-password"
            required
            className="web-settings-input"
          />
        </label>
        {error && <p className="web-login-error" role="alert">{error}</p>}
        <button
          type="submit"
          disabled={submitting}
          className="web-button-primary"
        >
          {submitting ? "Signing in…" : "Sign in"}
        </button>
      </form>
    </main>
  );
}
