import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";

type WebSettingsState = {
  llmApiKey: string;
  llmModel: string;
  llmProvider: string;
  llmBaseUrl: string;
  discogsToken: string;
  theAudioDbApiKey: string;
  lyricsDownloadEnabled: boolean;
  lyricsApiUrl: string;
  chineseScript: string;
  assistantAutonomous: boolean;
  debug: boolean;
};

type ProviderDefinition = {
  label: string;
  baseUrl: string;
  models: string[];
};

const PROVIDERS: Record<string, ProviderDefinition> = {
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
  opencode_go: {
    label: "OpenCode-Go",
    baseUrl: "http://localhost:8080/v1",
    models: ["default"],
  },
  opencode_zen: {
    label: "OpenCode-Zen",
    baseUrl: "http://localhost:7070/v1",
    models: ["default"],
  },
};

const PROVIDER_OPTIONS = [
  ["openai", "OpenAI"],
  ["claude", "Claude (Anthropic)"],
  ["openrouter", "OpenRouter"],
  ["opencode_go", "OpenCode-Go"],
  ["opencode_zen", "OpenCode-Zen"],
] as const;

const DEFAULT_SETTINGS: WebSettingsState = {
  llmApiKey: "",
  llmModel: "",
  llmProvider: "",
  llmBaseUrl: "",
  discogsToken: "",
  theAudioDbApiKey: "",
  lyricsDownloadEnabled: false,
  lyricsApiUrl: "",
  chineseScript: "",
  assistantAutonomous: false,
  debug: false,
};

const SECTIONS = [
  { id: "general", label: "General" },
  { id: "providers", label: "AI & Providers" },
  { id: "lyrics", label: "Lyrics & metadata" },
  { id: "assistant", label: "Assistant & diagnostics" },
] as const;

function settingString(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function settingBoolean(value: unknown): boolean {
  return value === true;
}

export interface WebSettingsPageProps {
  presentation?: "page" | "drawer";
  onClose?: () => void;
}

export function WebSettingsPage({
  presentation = "page",
  onClose,
}: WebSettingsPageProps) {
  const isDrawer = presentation === "drawer";
  const [settings, setSettings] = useState<WebSettingsState>(DEFAULT_SETTINGS);
  const [loading, setLoading] = useState(true);
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saveMessage, setSaveMessage] = useState<string | null>(null);
  const [appVersion, setAppVersion] = useState<string | null>(null);
  const [keyConfigured, setKeyConfigured] = useState(false);
  const [connection, setConnection] = useState<"idle" | "testing" | "success" | "error">("idle");
  const [connectionError, setConnectionError] = useState<string | null>(null);
  const [activeSection, setActiveSection] = useState<(typeof SECTIONS)[number]["id"]>("general");
  const loadGenerationRef = useRef(0);
  const drawerRef = useRef<HTMLDivElement | null>(null);

  const loadSettings = useCallback(async () => {
    const generation = ++loadGenerationRef.current;
    const isCurrent = () => loadGenerationRef.current === generation;
    setLoading(true);
    setLoaded(false);
    setError(null);
    setSaveMessage(null);
    try {
      const [config, info] = await Promise.all([
        window.api.getConfig(),
        window.api.appInfo().catch(() => null),
      ]);
      if (!isCurrent()) return;
      setSettings({
        llmApiKey: settingString(config.llmApiKey),
        llmModel: settingString(config.llmModel),
        llmProvider: settingString(config.llmProvider),
        llmBaseUrl: settingString(config.llmBaseUrl),
        discogsToken: settingString(config.discogsToken),
        theAudioDbApiKey: settingString(config.theAudioDbApiKey),
        lyricsDownloadEnabled: settingBoolean(config.lyricsDownloadEnabled),
        lyricsApiUrl: settingString(config.lyricsApiUrl),
        chineseScript: settingString(config.chineseScript),
        assistantAutonomous: settingBoolean(config.assistantAutonomous),
        debug: settingBoolean(config.debug),
      });
      setKeyConfigured(config.llmApiKeyConfigured === true || Boolean(config.llmApiKey));
      setAppVersion(info?.version ?? null);
      setLoaded(true);
    } catch (reason) {
      if (!isCurrent()) return;
      setError(reason instanceof Error ? reason.message : "Failed to load settings");
    } finally {
      if (isCurrent()) setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadSettings();
    return () => {
      loadGenerationRef.current += 1;
    };
  }, [loadSettings]);

  useEffect(() => {
    if (!isDrawer) return;
    const frame = requestAnimationFrame(() => {
      drawerRef.current?.querySelector<HTMLElement>("button:not([disabled]), input:not([disabled]), select:not([disabled])")?.focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [isDrawer]);

  const updateSetting = useCallback(<K extends keyof WebSettingsState>(key: K, value: WebSettingsState[K]) => {
    setSettings((current) => ({ ...current, [key]: value }));
    setSaveMessage(null);
    setConnection("idle");
    setConnectionError(null);
  }, []);

  const handleProviderChange = useCallback((provider: string) => {
    const definition = PROVIDERS[provider];
    setSettings((current) => ({
      ...current,
      llmProvider: provider,
      llmBaseUrl: definition?.baseUrl ?? "",
      llmModel: definition?.models[0] ?? current.llmModel,
    }));
    setSaveMessage(null);
    setConnection("idle");
    setConnectionError(null);
  }, []);

  const handleSave = useCallback(async () => {
    if (!loaded) return;
    setSaving(true);
    setError(null);
    setSaveMessage(null);
    try {
      const writes: Promise<void>[] = [];
      // Redacted values are renderer-safe placeholders. Never send them back
      // to the service, and never send an empty value that would clear a key.
      for (const [key, value] of [
        ["llmApiKey", settings.llmApiKey],
        ["discogsToken", settings.discogsToken],
        ["theAudioDbApiKey", settings.theAudioDbApiKey],
      ] as const) {
        if (value && !value.startsWith("****")) {
          writes.push(window.api.setConfig(key, value));
        }
      }
      writes.push(window.api.setConfig("llmModel", settings.llmModel));
      writes.push(window.api.setConfig("llmProvider", settings.llmProvider || null));
      writes.push(window.api.setConfig("llmBaseUrl", settings.llmBaseUrl || null));
      writes.push(window.api.setConfig("lyricsApiUrl", settings.lyricsApiUrl || null));
      writes.push(window.api.setConfig("lyricsDownloadEnabled", settings.lyricsDownloadEnabled));
      writes.push(window.api.setConfig("chineseScript", settings.chineseScript || null));
      writes.push(window.api.setConfig("assistantAutonomous", settings.assistantAutonomous));
      writes.push(window.api.setConfig("debug", settings.debug));
      writes.push(window.api.setDebugMode(settings.debug));
      await Promise.all(writes);
      setSaveMessage("Settings saved");
      onClose?.();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Failed to save settings");
    } finally {
      setSaving(false);
    }
  }, [loaded, onClose, settings]);

  const handleDrawerKeyDown = useCallback((event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      onClose?.();
      return;
    }
    if (event.key !== "Tab") return;
    const focusable = Array.from(drawerRef.current?.querySelectorAll<HTMLElement>(
      "button:not([disabled]), input:not([disabled]), select:not([disabled]), a[href]",
    ) ?? []).filter((element) => !element.hasAttribute("hidden") && !element.closest(".web-settings-save"));
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }, [onClose]);

  const handleTestConnection = useCallback(async () => {
    if (!settings.llmModel || (!keyConfigured && !settings.llmApiKey)) return;
    setConnection("testing");
    setConnectionError(null);
    try {
      // The service resolves a blank key from its protected config snapshot.
      const key = settings.llmApiKey.startsWith("****") ? "" : settings.llmApiKey;
      await window.api.testLlmConnection(
        key,
        settings.llmModel,
        settings.llmProvider || undefined,
        settings.llmBaseUrl || undefined,
      );
      setConnection("success");
    } catch (reason) {
      setConnection("error");
      setConnectionError(reason instanceof Error ? reason.message : "Connection failed");
    }
  }, [keyConfigured, settings.llmApiKey, settings.llmBaseUrl, settings.llmModel, settings.llmProvider]);

  const modelOptions = useMemo(() => PROVIDERS[settings.llmProvider]?.models ?? [], [settings.llmProvider]);
  const hasConfiguredKey = keyConfigured || Boolean(settings.llmApiKey);

  const settingsContent = (
    <>
      {!isDrawer && <header className="web-settings-heading web-page-heading">
        <div>
          <span className="web-visually-hidden">CONFIGURATION</span>
          <p className="web-eyebrow">Configuration</p>
          <h1 className="web-page-title">Settings</h1>
          <p className="web-page-description">Control metadata providers, lyrics, and editing preferences for this browser UI.</p>
        </div>
      </header>}

      {error && <div role="alert" className="mb-4 flex flex-wrap items-center justify-between gap-3 rounded-lg border border-[#f0c9d0] bg-[#fff7f8] px-4 py-3 text-sm text-[#a24557]"><span>{error}</span><button type="button" onClick={() => void loadSettings()} disabled={loading} className={SECONDARY_BUTTON_CLASS}>Retry</button></div>}

      <div data-testid="web-settings-detail" className="web-settings-detail">
        <aside className="web-settings-detail-rail">
          <p className="web-settings-detail-label">Settings</p>
          <nav aria-label="Settings sections" className="web-settings-tabs">
            {SECTIONS.map((section) => (
              <a
                key={section.id}
                href={`#${section.id}`}
                aria-current={activeSection === section.id ? "page" : undefined}
                className={`web-settings-tab ${activeSection === section.id ? "is-selected" : ""}`}
                onClick={(event) => {
                  event.preventDefault();
                  setActiveSection(section.id);
                  document.getElementById(section.id)?.scrollIntoView?.({ behavior: "smooth", block: "start" });
                }}
              >
                {section.label}
              </a>
            ))}
          </nav>
          <p className="web-settings-detail-note">Changes stay on the service until you save them.</p>
        </aside>

        <div className="web-settings-detail-content">
          <section id="general" className="web-settings-card">
            <CardHeading title="General" description="Service information for this browser runtime." />
            <div className="grid gap-3 px-4 py-4 sm:grid-cols-2">
              <InfoRow label="Runtime" value="Browser service" />
              <InfoRow label="Version" value={appVersion ? `Version ${appVersion}` : "Version unavailable"} />
              <InfoRow label="Credentials" value={hasConfiguredKey ? "Protected values configured" : "No provider key configured"} />
            </div>
          </section>

          <section id="providers" className="web-settings-card">
            <CardHeading title="AI & metadata providers" description="Choose the services that can suggest metadata. Secrets remain protected by the service." />
            <div className="web-settings-card-body">
              <Field label="Provider" htmlFor="web-settings-provider">
                <select id="web-settings-provider" value={settings.llmProvider} onChange={(event) => handleProviderChange(event.target.value)} className={INPUT_CLASS}>
                  <option value="">Select provider</option>
                  {PROVIDER_OPTIONS.map(([value, label]) => <option key={value} value={value}>{label}</option>)}
                </select>
              </Field>
              <Field label="Model" htmlFor="web-settings-model" hint="Use a provider model identifier supported by the service.">
                <input id="web-settings-model" list="web-settings-models" value={settings.llmModel} onChange={(event) => updateSetting("llmModel", event.target.value)} placeholder="gpt-4o-mini" className={INPUT_CLASS} />
                <datalist id="web-settings-models">{modelOptions.map((model) => <option key={model} value={model} />)}</datalist>
              </Field>
              <Field label="Base URL" htmlFor="web-settings-base-url" hint="Optional endpoint override for compatible providers.">
                <input id="web-settings-base-url" value={settings.llmBaseUrl} onChange={(event) => updateSetting("llmBaseUrl", event.target.value)} placeholder={PROVIDERS[settings.llmProvider]?.baseUrl ?? "https://api.openai.com/v1"} className={INPUT_CLASS} />
              </Field>
              <SecretField id="web-settings-llm-key" label="LLM API key" value={settings.llmApiKey} onChange={(value) => updateSetting("llmApiKey", value)} />
              <SecretField id="web-settings-discogs" label="Discogs token" value={settings.discogsToken} onChange={(value) => updateSetting("discogsToken", value)} />
              <SecretField id="web-settings-audiodb" label="TheAudioDB API key" value={settings.theAudioDbApiKey} onChange={(value) => updateSetting("theAudioDbApiKey", value)} />
              <div className="flex flex-wrap items-center gap-3 pt-1">
                <button type="button" onClick={() => void handleTestConnection()} disabled={connection === "testing" || !settings.llmModel || !hasConfiguredKey} className={SECONDARY_BUTTON_CLASS}>
                  {connection === "testing" ? "Testing…" : "Test connection"}
                </button>
                {connection === "success" && <span className="text-sm text-[#3d7549]">Connection successful</span>}
                {connection === "error" && <span role="alert" className="text-sm text-[#9b3e24]">{connectionError}</span>}
              </div>
            </div>
          </section>

          <section id="lyrics" className="web-settings-card">
            <CardHeading title="Lyrics & metadata" description="Keep lyrics and written tag text aligned with your existing native settings." />
            <div className="web-settings-card-body">
              <Field label="Lyrics API URL" htmlFor="web-settings-lyrics-url" hint="Leave blank to use the service default.">
                <input id="web-settings-lyrics-url" value={settings.lyricsApiUrl} onChange={(event) => updateSetting("lyricsApiUrl", event.target.value)} placeholder="https://lrclib.net/api" className={INPUT_CLASS} />
              </Field>
              <ToggleField id="web-settings-lyrics-download" label="Auto-download lyrics" description="Fetch lyrics when no local .lrc or .txt file is present." checked={settings.lyricsDownloadEnabled} onChange={(checked) => updateSetting("lyricsDownloadEnabled", checked)} />
              <Field label="Chinese script" htmlFor="web-settings-script" hint="Apply conversion when writing tag text.">
                <select id="web-settings-script" value={settings.chineseScript} onChange={(event) => updateSetting("chineseScript", event.target.value)} className={INPUT_CLASS}>
                  <option value="">Default (no conversion)</option>
                  <option value="simplified">Simplified Chinese</option>
                  <option value="traditional">Traditional Chinese</option>
                </select>
              </Field>
            </div>
          </section>

          <section id="assistant" className="web-settings-card">
            <CardHeading title="Assistant & diagnostics" description="Browser-supported assistant behavior and service diagnostics." />
            <div className="web-settings-card-body">
              <ToggleField id="web-settings-assistant" label="Auto-apply assistant actions" description="Apply low-risk tag changes without an extra approval step." checked={settings.assistantAutonomous} onChange={(checked) => updateSetting("assistantAutonomous", checked)} />
              <ToggleField id="web-settings-debug" label="Debug mode" description="Publish verbose service diagnostics to the browser activity stream." checked={settings.debug} onChange={(checked) => updateSetting("debug", checked)} />
              <p className="border-t border-[#ebe9e6] pt-4 text-xs leading-5 text-[#85807a]">Native update and desktop-only controls are intentionally unavailable in the browser service.</p>
            </div>
          </section>

          <div className="web-settings-save">
            <div className="text-sm text-[#77736f]">{saveMessage ?? "Save changes when you are ready."}</div>
            <button type="button" onClick={() => void handleSave()} disabled={loading || saving || !loaded} className={PRIMARY_BUTTON_CLASS}>
              {saving ? "Saving…" : "Save settings"}
            </button>
          </div>
        </div>
      </div>
    </>
  );

  if (isDrawer) {
    return (
      <div className="web-settings-drawer-backdrop" onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose?.();
      }}>
        <div ref={drawerRef} role="dialog" aria-modal="true" aria-label="Workspace settings" className="web-settings-drawer" onKeyDown={handleDrawerKeyDown}>
          <header className="web-settings-drawer-header">
            <h2>Workspace settings</h2>
            <div>
              <button type="button" className="web-button-primary" onClick={() => void handleSave()} disabled={loading || saving || !loaded}>{saving ? "Applying…" : "Apply"}</button>
              <button type="button" className="web-settings-drawer-cancel" onClick={onClose}>Cancel</button>
            </div>
          </header>
          <div className="web-settings-drawer-body">{settingsContent}</div>
        </div>
      </div>
    );
  }

  return <div data-testid="web-settings-page" className="web-settings-page">{settingsContent}</div>;
}

function CardHeading({ title, description }: { title: string; description: string }) {
  return <div className="web-settings-card-heading"><h2>{title}</h2><p>{description}</p></div>;
}

function InfoRow({ label, value }: { label: string; value: string }) {
  return <div className="web-settings-info"><p className="web-settings-info-label">{label}</p><p className="web-settings-info-value">{value}</p></div>;
}

function Field({ label, htmlFor, hint, children }: { label: string; htmlFor: string; hint?: string; children: React.ReactNode }) {
  return <div className="web-settings-field"><label htmlFor={htmlFor} className="web-settings-label">{label}</label>{children}{hint && <p className="web-settings-hint">{hint}</p>}</div>;
}

function SecretField({ id, label, value, onChange }: { id: string; label: string; value: string; onChange: (value: string) => void }) {
  return <Field label={label} htmlFor={id} hint={value.startsWith("****") ? "A masked value is already configured. Leave this blank to preserve it." : "Leave blank to keep the current protected value."}><input id={id} type="password" autoComplete="off" value={value} onChange={(event) => onChange(event.target.value)} placeholder="Enter a new value to replace the current one" className={INPUT_CLASS} /></Field>;
}

function ToggleField({ id, label, description, checked, onChange }: { id: string; label: string; description: string; checked: boolean; onChange: (checked: boolean) => void }) {
  return <div className="flex items-start justify-between gap-5 border-t border-[#e7e8ee] pt-4 first:border-t-0 first:pt-0"><div><label htmlFor={id} className="web-settings-label">{label}</label><p className="web-settings-hint">{description}</p></div><button id={id} type="button" role="switch" aria-checked={checked} onClick={() => onChange(!checked)} className={`relative mt-0.5 inline-flex h-6 w-11 shrink-0 rounded-full transition-colors focus:outline-none ${checked ? "bg-[#9b7cf7]" : "bg-[#d9dbe3]"}`}><span className={`absolute top-1 h-4 w-4 rounded-full bg-white shadow-sm transition-transform ${checked ? "translate-x-6" : "translate-x-1"}`} /></button></div>;
}

const INPUT_CLASS = "web-settings-input";
const PRIMARY_BUTTON_CLASS = "web-button-primary";
const SECONDARY_BUTTON_CLASS = "web-button-secondary";
