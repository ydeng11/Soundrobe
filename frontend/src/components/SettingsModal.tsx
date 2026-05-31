import React, { useState, useEffect } from "react";

interface SettingsState {
  llmApiKey: string;
  llmModel: string;
  remoteLookupEnabled: boolean;
  discogsEnabled: boolean;
  discogsToken: string;
  debug: boolean;
  lyricsDownloadEnabled: boolean;
  lyricsApiUrl: string;
  assistantAutonomous: boolean;
}

interface SettingsModalProps {
  open: boolean;
  onClose: () => void;
}

export function SettingsModal({ open, onClose }: SettingsModalProps) {
  const [settings, setSettings] = useState<SettingsState>({
    llmApiKey: "",
    llmModel: "",
    remoteLookupEnabled: true,
    discogsEnabled: true,
    discogsToken: "",
    debug: false,
    lyricsDownloadEnabled: false,
    lyricsApiUrl: "",
    assistantAutonomous: false,
  });
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setLoading(true);
    setSaveError(null);

    (async () => {
      try {
        const cfg = await window.api.getConfig();
        setSettings({
          llmApiKey: "",
          llmModel: (cfg.llmModel as string) ?? "",
          remoteLookupEnabled: (cfg.remoteLookupEnabled as boolean) ?? true,
          discogsEnabled: (cfg.discogsEnabled as boolean) ?? true,
          lyricsDownloadEnabled: (cfg.lyricsDownloadEnabled as boolean) ?? false,
          lyricsApiUrl: (cfg.lyricsApiUrl as string) ?? "",
          discogsToken: "",
          debug: (cfg.debug as boolean) ?? false,
          assistantAutonomous: (cfg.assistantAutonomous as boolean) ?? false,
        });
      } catch (err) {
        setSaveError(
          err instanceof Error ? err.message : "Failed to load config",
        );
      } finally {
        setLoading(false);
      }
    })();
  }, [open]);

  const handleSave = async () => {
    setSaving(true);
    setSaveError(null);

    try {
      const promises: Promise<void>[] = [];
      if (settings.llmApiKey) {
        promises.push(window.api.setConfig("llmApiKey", settings.llmApiKey));
      }
      if (settings.discogsToken) {
        promises.push(
          window.api.setConfig("discogsToken", settings.discogsToken),
        );
      }
      promises.push(window.api.setConfig("llmModel", settings.llmModel));
      promises.push(
        window.api.setConfig("remoteLookupEnabled", settings.remoteLookupEnabled),
      );
      promises.push(window.api.setConfig("discogsEnabled", settings.discogsEnabled));
      promises.push(window.api.setConfig("lyricsDownloadEnabled", settings.lyricsDownloadEnabled));
      promises.push(window.api.setConfig("lyricsApiUrl", settings.lyricsApiUrl || null));
      promises.push(window.api.setConfig("debug", settings.debug));
      promises.push(window.api.setDebugMode(settings.debug));
      promises.push(window.api.setConfig("assistantAutonomous", settings.assistantAutonomous));

      await Promise.all(promises);
      onClose();
    } catch (err) {
      setSaveError(
        err instanceof Error ? err.message : "Failed to save config",
      );
    } finally {
      setSaving(false);
    }
  };

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/20 backdrop-blur-sm">
      <div className="w-[400px] rounded-xl bg-white border border-border shadow-xl shadow-black/10">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-3.5 border-b border-border/60">
          <div className="flex items-center gap-2.5">
            <svg
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="none"
              stroke="#007aff"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <circle cx="12" cy="12" r="3" />
              <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" />
            </svg>
            <h2 className="text-[13px] font-semibold text-text-primary">Settings</h2>
          </div>
          <button
            onClick={onClose}
            className="inline-flex items-center justify-center w-7 h-7 text-text-muted hover:text-text-primary rounded-lg hover:bg-surface-hover transition-all"
          >
            <svg
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <path d="M18 6 6 18" />
              <path d="m6 6 12 12" />
            </svg>
          </button>
        </div>

        {/* Body */}
        <div className="px-5 py-4 space-y-4">
          {loading ? (
            <div className="flex items-center justify-center py-8">
              <div className="flex items-center gap-2 text-text-muted text-[12px]">
                <svg
                  width="16"
                  height="16"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.5"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  className="animate-spin"
                >
                  <line x1="12" y1="2" x2="12" y2="6" />
                  <line x1="12" y1="18" x2="12" y2="22" />
                  <line x1="4.93" y1="4.93" x2="7.76" y2="7.76" />
                  <line x1="16.24" y1="16.24" x2="19.07" y2="19.07" />
                  <line x1="2" y1="12" x2="6" y2="12" />
                  <line x1="18" y1="12" x2="22" y2="12" />
                  <line x1="4.93" y1="19.07" x2="7.76" y2="16.24" />
                  <line x1="16.24" y1="7.76" x2="19.07" y2="4.93" />
                </svg>
                Loading…
              </div>
            </div>
          ) : (
            <>
              {saveError && <ErrorBanner message={saveError} />}

              <FieldRow label="LLM API Key" description="OpenRouter or compatible API key">
                <InputField
                  type="password"
                  value={settings.llmApiKey}
                  onChange={(v) => setSettings({ ...settings, llmApiKey: v })}
                  placeholder="sk-or-v1-… (leave blank to keep current)"
                />
                <p className="text-[10px] text-text-muted/60 mt-1">
                  Can also be set via the <code className="text-[10px] bg-surface-alt/50 px-1 rounded">LLM_API_KEY</code> env var
                </p>
              </FieldRow>

              <FieldRow label="LLM Model" description="Provider/model identifier (e.g. openrouter/owl-alpha)">
                <InputField
                  value={settings.llmModel}
                  onChange={(v) => setSettings({ ...settings, llmModel: v })}
                  placeholder="openrouter/owl-alpha"
                />
                <p className="text-[10px] text-text-muted/60 mt-1">
                  Can also be set via the <code className="text-[10px] bg-surface-alt/50 px-1 rounded">LLM_MODEL</code> env var
                </p>
              </FieldRow>

              <FieldRow label="Discogs Token" description="Personal access token for Discogs API">
                <InputField
                  type="password"
                  value={settings.discogsToken}
                  onChange={(v) => setSettings({ ...settings, discogsToken: v })}
                  placeholder="(leave blank to keep current)"
                />
              </FieldRow>

              <hr className="border-border/40 my-2" />
              <h3 className="text-[11px] font-semibold text-text-primary tracking-wide uppercase">Lyrics</h3>

              <FieldRow label="Lyrics API URL" description="Base URL for lyrics API (e.g. https://lrclib.net/api)">
                <InputField
                  value={settings.lyricsApiUrl}
                  onChange={(v) => setSettings({ ...settings, lyricsApiUrl: v })}
                  placeholder="https://lrclib.net/api"
                />
              </FieldRow>

              <div className="pt-1 space-y-3">
                <ToggleRow
                  label="Auto-download Lyrics"
                  description="Download lyrics from API when no local .lrc/.txt file exists"
                  checked={settings.lyricsDownloadEnabled}
                  onChange={(v) => setSettings({ ...settings, lyricsDownloadEnabled: v })}
                />
                <ToggleRow
                  label="Remote Lookup"
                  description="Search MusicBrainz &amp; Discogs when dataset misses"
                  checked={settings.remoteLookupEnabled}
                  onChange={(v) => setSettings({ ...settings, remoteLookupEnabled: v })}
                />
                <ToggleRow
                  label="Discogs"
                  description="Enable Discogs as a lookup source"
                  checked={settings.discogsEnabled}
                  onChange={(v) => setSettings({ ...settings, discogsEnabled: v })}
                />
                <ToggleRow
                  label="Auto-apply Assistant Actions"
                  description="When enabled, the assistant applies low-risk tag changes without manual approval. Medium/high risk actions still require confirmation."
                  checked={settings.assistantAutonomous}
                  onChange={(v) => setSettings({ ...settings, assistantAutonomous: v })}
                />
                <ToggleRow
                  label="Debug Mode"
                  description="Verbose logging to DevTools console and ~/.auto-tagger/auto-tag-debug-*.log"
                  checked={settings.debug}
                  onChange={(v) => setSettings({ ...settings, debug: v })}
                />
              </div>
            </>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-end gap-2 px-5 py-3.5 border-t border-border/60 bg-surface-alt/30">
          <button
            onClick={onClose}
            className="px-4 py-1.5 text-[11.5px] font-medium rounded-lg text-text-secondary hover:text-text-primary hover:bg-surface-hover transition-all"
          >
            Cancel
          </button>
          <button
            onClick={handleSave}
            disabled={loading || saving}
            className={`px-4 py-1.5 text-[11.5px] font-medium rounded-lg transition-all ${
              loading || saving
                ? "bg-accent/20 text-accent/60 cursor-not-allowed"
                : "bg-accent text-white hover:bg-accent/90 shadow-sm active:scale-[0.97]"
            }`}
          >
            {saving ? (
              <span className="flex items-center gap-1.5">
                <svg
                  width="12"
                  height="12"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  className="animate-spin"
                >
                  <line x1="12" y1="2" x2="12" y2="6" />
                  <line x1="12" y1="18" x2="12" y2="22" />
                </svg>
                Saving…
              </span>
            ) : (
              "Save"
            )}
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Helpers ──────────────────────────────────────────────────────

const INPUT_CLASS =
  "w-full bg-white border border-border rounded-lg px-3 py-1.5 text-[12px] text-text-primary placeholder-text-muted/40 outline-none transition-all focus:border-accent/60 focus:shadow-[0_0_0_3px_rgba(0,122,255,0.2)]";

function FieldRow({
  label,
  description,
  children,
}: {
  label: string;
  description?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-baseline justify-between">
        <label className="text-[11px] font-medium text-text-primary">{label}</label>
        {description && <span className="text-[10px] text-text-muted">{description}</span>}
      </div>
      {children}
    </div>
  );
}

function InputField({
  value,
  onChange,
  placeholder,
  type,
}: {
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  type?: string;
}) {
  return (
    <input
      type={type ?? "text"}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      className={INPUT_CLASS}
    />
  );
}

function ToggleRow({
  label,
  description,
  checked,
  onChange,
}: {
  label: string;
  description: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div className="flex items-center justify-between py-0.5">
      <div className="flex flex-col gap-0.5">
        <span className="text-[12px] font-medium text-text-primary">{label}</span>
        <span className="text-[10px] text-text-muted leading-tight">{description}</span>
      </div>
      <button
        onClick={() => onChange(!checked)}
        className={`relative inline-flex h-5 w-9 shrink-0 rounded-full transition-colors ${
          checked ? "bg-accent" : "bg-border"
        }`}
      >
        <span
          className={`absolute top-0.5 left-0.5 inline-block h-4 w-4 rounded-full bg-white shadow-sm transition-transform ${
            checked ? "translate-x-4" : "translate-x-0"
          }`}
        />
      </button>
    </div>
  );
}

function ErrorBanner({ message }: { message: string }) {
  return (
    <div className="bg-red-50 border border-red-200 rounded-lg px-3.5 py-2.5 text-[11px] text-[#ff3b30] flex items-center gap-2">
      <svg
        width="14"
        height="14"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
        className="shrink-0"
      >
        <circle cx="12" cy="12" r="10" />
        <line x1="12" y1="8" x2="12" y2="12" />
        <line x1="12" y1="16" x2="12.01" y2="16" />
      </svg>
      {message}
    </div>
  );
}
