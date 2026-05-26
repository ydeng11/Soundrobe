import React, { useState, useEffect } from "react";

interface SettingsState {
  llmApiKey: string;
  llmModel: string;
  remoteLookupEnabled: boolean;
  discogsEnabled: boolean;
  discogsToken: string;
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
  });
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  // Load current config when the modal opens
  useEffect(() => {
    if (!open) return;
    setLoading(true);
    setSaveError(null);

    (async () => {
      try {
        const cfg = await window.api.getConfig();

        setSettings({
          llmApiKey: "",
          llmModel: (cfg.llmModel as string) ?? "deepseek/deepseek-chat:free",
          remoteLookupEnabled: (cfg.remoteLookupEnabled as boolean) ?? true,
          discogsEnabled: (cfg.discogsEnabled as boolean) ?? true,
          discogsToken: "",
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
      // Only save fields that changed
      if (settings.llmApiKey) {
        await window.api.setConfig("llmApiKey", settings.llmApiKey);
      }
      if (settings.discogsToken) {
        await window.api.setConfig("discogsToken", settings.discogsToken);
      }
      await window.api.setConfig("llmModel", settings.llmModel);
      await window.api.setConfig(
        "remoteLookupEnabled",
        settings.remoteLookupEnabled,
      );
      await window.api.setConfig("discogsEnabled", settings.discogsEnabled);

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
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div className="w-96 rounded-lg bg-surface border border-gray-700/50 shadow-2xl">
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-gray-700/30">
          <h2 className="text-sm font-semibold text-text-primary">
            ⚙️ Settings
          </h2>
          <button
            onClick={onClose}
            className="text-text-muted hover:text-text-primary transition-colors text-xs px-1 py-0.5 rounded hover:bg-surface-hover"
          >
            ✕
          </button>
        </div>

        {/* Body */}
        <div className="px-4 py-3 space-y-3">
          {loading ? (
            <div className="text-text-muted text-xs text-center py-4">
              Loading…
            </div>
          ) : (
            <>
              {saveError && (
                <div className="bg-red-900/20 border border-red-800/30 rounded px-3 py-2 text-red-400 text-[10px]">
                  {saveError}
                </div>
              )}

              {/* LLM API Key */}
              <FieldRow label="LLM API Key">
                <input
                  type="password"
                  value={settings.llmApiKey}
                  onChange={(e) =>
                    setSettings({ ...settings, llmApiKey: e.target.value })
                  }
                  placeholder="sk-or-v1-… (leave blank to keep current)"
                  className="flex-1 bg-surface/60 border border-gray-700/40 rounded px-2 py-1 text-[11px] text-text-primary placeholder-text-muted/40 outline-none focus:border-accent/40 transition-colors"
                />
              </FieldRow>

              {/* LLM Model */}
              <FieldRow label="LLM Model">
                <input
                  type="text"
                  value={settings.llmModel}
                  onChange={(e) =>
                    setSettings({ ...settings, llmModel: e.target.value })
                  }
                  placeholder="deepseek/deepseek-chat:free"
                  className="flex-1 bg-surface/60 border border-gray-700/40 rounded px-2 py-1 text-[11px] text-text-primary placeholder-text-muted/40 outline-none focus:border-accent/40 transition-colors"
                />
              </FieldRow>

              {/* Discogs Token */}
              <FieldRow label="Discogs Token">
                <input
                  type="password"
                  value={settings.discogsToken}
                  onChange={(e) =>
                    setSettings({ ...settings, discogsToken: e.target.value })
                  }
                  placeholder="(leave blank to keep current)"
                  className="flex-1 bg-surface/60 border border-gray-700/40 rounded px-2 py-1 text-[11px] text-text-primary placeholder-text-muted/40 outline-none focus:border-accent/40 transition-colors"
                />
              </FieldRow>

              {/* Remote Lookup */}
              <ToggleRow
                label="Remote Lookup"
                description="Search MusicBrainz & Discogs when dataset misses"
                checked={settings.remoteLookupEnabled}
                onChange={(v) =>
                  setSettings({ ...settings, remoteLookupEnabled: v })
                }
              />

              {/* Discogs */}
              <ToggleRow
                label="Discogs"
                description="Enable Discogs as a lookup source"
                checked={settings.discogsEnabled}
                onChange={(v) =>
                  setSettings({ ...settings, discogsEnabled: v })
                }
              />
            </>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-end gap-2 px-4 py-3 border-t border-gray-700/30">
          <button
            onClick={onClose}
            className="px-3 py-1.5 text-[10px] font-medium rounded text-text-muted hover:text-text-primary hover:bg-surface-hover transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={handleSave}
            disabled={loading || saving}
            className={`px-3 py-1.5 text-[10px] font-medium rounded transition-colors ${
              loading || saving
                ? "bg-accent/15 text-accent-light/50 cursor-not-allowed"
                : "bg-accent/25 text-accent-light hover:bg-accent/35"
            }`}
          >
            {saving ? "Saving…" : "Save"}
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Helpers ──────────────────────────────────────────────────────

function FieldRow({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1">
      <label className="text-[10px] font-medium text-text-muted uppercase tracking-wider">
        {label}
      </label>
      {children}
    </div>
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
    <div className="flex items-center justify-between">
      <div className="flex flex-col gap-0.5">
        <span className="text-[11px] font-medium text-text-primary">
          {label}
        </span>
        <span className="text-[9px] text-text-muted">{description}</span>
      </div>
      <button
        onClick={() => onChange(!checked)}
        className={`relative w-8 h-4 rounded-full transition-colors ${
          checked ? "bg-accent/60" : "bg-gray-700/50"
        }`}
      >
        <span
          className={`absolute top-0.5 w-3 h-3 rounded-full bg-white transition-transform ${
            checked ? "translate-x-4" : "translate-x-0.5"
          }`}
        />
      </button>
    </div>
  );
}
