/**
 * Tag source configuration for the browser runtime.
 *
 * Music Tag Web exposes a `插件管理` (tag source plugin) page. Soundrobe's
 * equivalent is its metadata source configuration: MusicBrainz needs no
 * credentials, Discogs and TheAudioDB take tokens, and the bounded AI fallback
 * takes an LLM provider.
 *
 * Secret handling follows the renderer contract:
 *  - the service only ever returns redacted secret values (`****…`),
 *  - a redacted or empty value is never written back, so saving an unrelated
 *    field cannot clear a stored credential,
 *  - testing a connection with a redacted key sends a blank key and lets the
 *    service resolve it from its protected snapshot.
 */

import React from "react";

export interface ProviderDefinition {
  label: string;
  baseUrl: string;
  models: string[];
}

/** AI providers that can serve the metadata fallback. */
const AI_PROVIDERS: Record<string, ProviderDefinition> = {
  openai: {
    label: "OpenAI",
    baseUrl: "https://api.openai.com/v1",
    models: ["gpt-4o", "gpt-4o-mini", "gpt-4-turbo"],
  },
  claude: {
    label: "Claude (Anthropic)",
    baseUrl: "https://api.anthropic.com/v1",
    models: ["claude-3-5-sonnet-20241022", "claude-3-opus-20240229"],
  },
  openrouter: {
    label: "OpenRouter",
    baseUrl: "https://openrouter.ai/api/v1",
    models: ["openai/gpt-4o", "openai/gpt-4o-mini", "anthropic/claude-3-5-sonnet"],
  },
};

export interface TagSourceSettings {
  discogsToken: string;
  theAudioDbApiKey: string;
  llmProvider: string;
  llmModel: string;
  llmBaseUrl: string;
  llmApiKey: string;
}

export const EMPTY_TAG_SOURCES: TagSourceSettings = {
  discogsToken: "",
  theAudioDbApiKey: "",
  llmProvider: "",
  llmModel: "",
  llmBaseUrl: "",
  llmApiKey: "",
};

function settingString(value: unknown): string {
  return typeof value === "string" ? value : "";
}

/** True when the service returned a redacted placeholder rather than a value. */
export function isRedacted(value: string): boolean {
  return value.startsWith("****");
}

/**
 * Build the `setConfig` writes for a save.
 *
 * Secret keys are only written when the operator entered a new value: a
 * redacted placeholder or an empty string must never reach the service, or
 * saving an unrelated field would erase a stored credential.
 */
export function buildTagSourceWrites(
  settings: TagSourceSettings,
): Array<[string, unknown]> {
  const writes: Array<[string, unknown]> = [];
  for (const [key, value] of [
    ["discogsToken", settings.discogsToken],
    ["theAudioDbApiKey", settings.theAudioDbApiKey],
    ["llmApiKey", settings.llmApiKey],
  ] as const) {
    if (value && !isRedacted(value)) {
      writes.push([key, value]);
    }
  }
  writes.push(["llmProvider", settings.llmProvider || null]);
  writes.push(["llmModel", settings.llmModel || null]);
  writes.push(["llmBaseUrl", settings.llmBaseUrl || null]);
  return writes;
}

/**
 * The key to present to a connection test. A redacted key becomes an empty
 * string so the service resolves the stored credential itself.
 */
export function connectionTestKey(llmApiKey: string): string {
  return isRedacted(llmApiKey) ? "" : llmApiKey;
}

export interface TagSourcesViewProps {
  api?: Window["api"];
}

export function TagSourcesView({ api = window.api }: TagSourcesViewProps) {
  const [settings, setSettings] = React.useState<TagSourceSettings>(EMPTY_TAG_SOURCES);
  const [loaded, setLoaded] = React.useState(false);
  const [saving, setSaving] = React.useState(false);
  const [message, setMessage] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [connection, setConnection] = React.useState<
    "idle" | "testing" | "success" | "error"
  >("idle");
  const [connectionError, setConnectionError] = React.useState<string | null>(null);

  const load = React.useCallback(async () => {
    try {
      const config = (await api.getConfig()) as Record<string, unknown>;
      setSettings({
        discogsToken: settingString(config.discogsToken),
        theAudioDbApiKey: settingString(config.theAudioDbApiKey),
        llmProvider: settingString(config.llmProvider),
        llmModel: settingString(config.llmModel),
        llmBaseUrl: settingString(config.llmBaseUrl),
        llmApiKey: settingString(config.llmApiKey),
      });
      setLoaded(true);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  }, [api]);

  React.useEffect(() => {
    void load();
  }, [load]);

  const update = (key: keyof TagSourceSettings, value: string) => {
    setSettings((current) => ({ ...current, [key]: value }));
    setMessage(null);
    setConnection("idle");
    setConnectionError(null);
  };

  const save = async () => {
    if (!loaded) return;
    setSaving(true);
    setError(null);
    setMessage(null);
    try {
      for (const [key, value] of buildTagSourceWrites(settings)) {
        await api.setConfig(key, value);
      }
      setMessage("Tag sources saved");
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setSaving(false);
    }
  };

  const testConnection = async () => {
    if (!settings.llmModel) return;
    setConnection("testing");
    setConnectionError(null);
    try {
      await api.testLlmConnection(
        connectionTestKey(settings.llmApiKey),
        settings.llmModel,
        settings.llmProvider || undefined,
        settings.llmBaseUrl || undefined,
      );
      setConnection("success");
    } catch (reason) {
      setConnection("error");
      setConnectionError(reason instanceof Error ? reason.message : "Connection failed");
    }
  };

  const models = AI_PROVIDERS[settings.llmProvider]?.models ?? [];

  return (
    <div className="sr-pane" data-testid="tag-sources">
      <div className="sr-toolbar">
        <h2 className="sr-section__title">Tag sources</h2>
        <div className="sr-header__spacer" />
        <button
          type="button"
          className="sr-btn sr-btn--primary"
          disabled={!loaded || saving}
          onClick={() => void save()}
        >
          {saving ? "Saving…" : "Save"}
        </button>
      </div>

      {error && (
        <p className="sr-banner sr-banner--error" role="alert">
          {error}
        </p>
      )}
      {message && <p className="sr-banner sr-banner--notice">{message}</p>}

      <div className="sr-sources">
        <section className="sr-section">
          <h3 className="sr-section__title">Metadata sources</h3>

          <div className="sr-source">
            <div className="sr-source__head">
              <span className="sr-source__name">MusicBrainz</span>
              <span className="sr-badge sr-badge--success">Always on</span>
            </div>
            <p className="sr-source__note">
              Free release data with no credentials required.
            </p>
          </div>

          <div className="sr-source">
            <div className="sr-source__head">
              <span className="sr-source__name">Discogs</span>
              <span
                className={`sr-badge ${settings.discogsToken ? "sr-badge--success" : "sr-badge--muted"}`}
              >
                {settings.discogsToken ? "Configured" : "Not configured"}
              </span>
            </div>
            <label className="sr-field sr-field--stacked">
              <span className="sr-field__label">Personal access token</span>
              <input
                className="sr-input"
                type="password"
                aria-label="Discogs token"
                value={settings.discogsToken}
                onChange={(event) => update("discogsToken", event.target.value)}
              />
            </label>
          </div>

          <div className="sr-source">
            <div className="sr-source__head">
              <span className="sr-source__name">TheAudioDB</span>
              <span
                className={`sr-badge ${settings.theAudioDbApiKey ? "sr-badge--success" : "sr-badge--muted"}`}
              >
                {settings.theAudioDbApiKey ? "Configured" : "Not configured"}
              </span>
            </div>
            <label className="sr-field sr-field--stacked">
              <span className="sr-field__label">API key</span>
              <input
                className="sr-input"
                type="password"
                aria-label="TheAudioDB API key"
                value={settings.theAudioDbApiKey}
                onChange={(event) => update("theAudioDbApiKey", event.target.value)}
              />
            </label>
          </div>
        </section>

        <section className="sr-section">
          <h3 className="sr-section__title">AI fallback</h3>
          <p className="sr-source__note">
            Used only for the bounded fallback after folder evidence, exact
            provider IDs, MusicBrainz and Discogs.
          </p>

          <label className="sr-field sr-field--stacked">
            <span className="sr-field__label">Provider</span>
            <select
              className="sr-input"
              aria-label="AI provider"
              value={settings.llmProvider}
              onChange={(event) => {
                const provider = event.target.value;
                const definition = AI_PROVIDERS[provider];
                setSettings((current) => ({
                  ...current,
                  llmProvider: provider,
                  llmBaseUrl: definition?.baseUrl ?? "",
                  llmModel: definition?.models[0] ?? current.llmModel,
                }));
                setMessage(null);
                setConnection("idle");
              }}
            >
              <option value="">Select provider</option>
              {Object.entries(AI_PROVIDERS).map(([key, definition]) => (
                <option key={key} value={key}>
                  {definition.label}
                </option>
              ))}
            </select>
          </label>

          <label className="sr-field sr-field--stacked">
            <span className="sr-field__label">Model</span>
            <input
              className="sr-input"
              aria-label="AI model"
              list="sr-ai-models"
              value={settings.llmModel}
              onChange={(event) => update("llmModel", event.target.value)}
            />
            <datalist id="sr-ai-models">
              {models.map((model) => (
                <option key={model} value={model} />
              ))}
            </datalist>
          </label>

          <label className="sr-field sr-field--stacked">
            <span className="sr-field__label">Base URL</span>
            <input
              className="sr-input"
              aria-label="AI base URL"
              value={settings.llmBaseUrl}
              onChange={(event) => update("llmBaseUrl", event.target.value)}
            />
          </label>

          <label className="sr-field sr-field--stacked">
            <span className="sr-field__label">API key</span>
            <input
              className="sr-input"
              type="password"
              aria-label="AI API key"
              value={settings.llmApiKey}
              onChange={(event) => update("llmApiKey", event.target.value)}
            />
          </label>

          <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
            <button
              type="button"
              className="sr-btn"
              disabled={connection === "testing" || !settings.llmModel}
              onClick={() => void testConnection()}
            >
              {connection === "testing" ? "Testing…" : "Test connection"}
            </button>
            {connection === "success" && (
              <span className="sr-badge sr-badge--success">Connection OK</span>
            )}
            {connection === "error" && (
              <span className="sr-badge sr-badge--danger">
                {connectionError ?? "Connection failed"}
              </span>
            )}
          </div>
        </section>
      </div>
    </div>
  );
}
