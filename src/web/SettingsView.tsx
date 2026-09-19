/**
 * Settings for the browser runtime.
 *
 * Mirrors the reference product's `系统设置` page, which is split into an
 * application section and a background-scraping section. Soundrobe's
 * equivalents are its real configuration keys (the same ones the desktop
 * settings screen writes) and the background-tagging schedule that drives
 * unattended runs.
 *
 * Schedule presets are copied from the reference's `配置扫描周期` dialog,
 * including its custom-crontab escape hatch. The crontab is validated by the
 * service, so an invalid expression is refused with the reason rather than
 * stored and silently never fired.
 */

import React from "react";
import type { AppInfo } from "../shared/desktop-api";
import {
  getBackgroundSettings,
  setBackgroundSettings,
  type BackgroundTaggingSettings,
} from "../shared/web-adapter";

export interface SettingsViewProps {
  /** Injected for tests; defaults to the installed `window.api` facade. */
  loadBackground?: () => Promise<BackgroundTaggingSettings>;
  saveBackground?: (
    patch: Partial<BackgroundTaggingSettings>,
  ) => Promise<BackgroundTaggingSettings>;
  loadConfig?: () => Promise<Record<string, unknown>>;
  saveConfig?: (key: string, value: unknown) => Promise<void>;
  loadAppInfo?: () => Promise<AppInfo>;
  setDebugMode?: (enabled: boolean) => Promise<void>;
}

/** Verbatim from the reference's quick presets. */
export const SCHEDULE_PRESETS: Array<{ label: string; value: string; hint: string }> = [
  { label: "Every minute", value: "*/1 * * * *", hint: "Runs once per minute" },
  { label: "Every 3 minutes", value: "*/3 * * * *", hint: "Runs every 3 minutes" },
  { label: "Every 10 minutes", value: "*/10 * * * *", hint: "Runs every 10 minutes" },
  { label: "Hourly", value: "0 * * * *", hint: "Runs on the hour" },
  { label: "Every 2 hours", value: "0 */2 * * *", hint: "Runs every 2 hours" },
  { label: "Daily at midnight", value: "0 0 * * *", hint: "Runs at 00:00 each day" },
  { label: "Custom", value: "custom", hint: "Use your own crontab expression" },
];

const CHINESE_SCRIPT_OPTIONS = [
  { value: "", label: "Leave unchanged" },
  { value: "simplified", label: "Simplified Chinese" },
  { value: "traditional", label: "Traditional Chinese" },
];

function configBool(config: Record<string, unknown>, key: string): boolean {
  return config[key] === true;
}

function configString(config: Record<string, unknown>, key: string): string {
  const value = config[key];
  return typeof value === "string" ? value : "";
}

export function SettingsView({
  loadBackground = getBackgroundSettings,
  saveBackground = setBackgroundSettings,
  loadConfig = () => window.api.getConfig(),
  saveConfig = (key, value) => window.api.setConfig(key, value),
  loadAppInfo = () => window.api.appInfo(),
  setDebugMode = (enabled) => window.api.setDebugMode(enabled),
}: SettingsViewProps) {
  const [background, setBackground] = React.useState<BackgroundTaggingSettings | null>(null);
  const [scheduleDraft, setScheduleDraft] = React.useState("0 0 * * *");
  const [preset, setPreset] = React.useState("0 0 * * *");
  const [directoryDraft, setDirectoryDraft] = React.useState("");
  const [lyricsUrlDraft, setLyricsUrlDraft] = React.useState("");
  const [config, setConfig] = React.useState<Record<string, unknown>>({});
  const [appInfo, setAppInfo] = React.useState<AppInfo | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [saved, setSaved] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState(false);

  React.useEffect(() => {
    let active = true;
    void (async () => {
      try {
        const [settings, stored] = await Promise.all([loadBackground(), loadConfig()]);
        if (!active) return;
        setBackground(settings);
        setScheduleDraft(settings.schedule);
        setPreset(
          SCHEDULE_PRESETS.some((option) => option.value === settings.schedule)
            ? settings.schedule
            : "custom",
        );
        setConfig(stored);
        setLyricsUrlDraft(configString(stored, "lyricsApiUrl"));
      } catch (reason) {
        if (active) setError(reason instanceof Error ? reason.message : String(reason));
      }
    })();
    return () => {
      active = false;
    };
  }, [loadBackground, loadConfig]);

  // Runtime details are informational; a failure here must not blank the page.
  React.useEffect(() => {
    let active = true;
    void (async () => {
      try {
        const info = await loadAppInfo();
        if (active) setAppInfo(info);
      } catch {
        /* reported by the sections that matter */
      }
    })();
    return () => {
      active = false;
    };
  }, [loadAppInfo]);

  const toggleDebug = React.useCallback(
    async (enabled: boolean) => {
      setBusy(true);
      setError(null);
      setSaved(null);
      try {
        await setDebugMode(enabled);
        setConfig((current) => ({ ...current, debug: enabled }));
        setSaved(enabled ? "Debug logging enabled" : "Debug logging disabled");
      } catch (reason) {
        setError(reason instanceof Error ? reason.message : String(reason));
      } finally {
        setBusy(false);
      }
    },
    [setDebugMode],
  );

  const persist = React.useCallback(
    async (patch: Partial<BackgroundTaggingSettings>, message: string) => {
      setBusy(true);
      setError(null);
      setSaved(null);
      try {
        const next = await saveBackground(patch);
        setBackground(next);
        setScheduleDraft(next.schedule);
        setSaved(message);
      } catch (reason) {
        // The service rejects an unusable crontab; surface its wording.
        setError(reason instanceof Error ? reason.message : String(reason));
      } finally {
        setBusy(false);
      }
    },
    [saveBackground],
  );

  const persistConfig = React.useCallback(
    async (key: string, value: unknown, message: string) => {
      setBusy(true);
      setError(null);
      setSaved(null);
      try {
        await saveConfig(key, value);
        setConfig((current) => ({ ...current, [key]: value }));
        setSaved(message);
      } catch (reason) {
        setError(reason instanceof Error ? reason.message : String(reason));
      } finally {
        setBusy(false);
      }
    },
    [saveConfig],
  );

  const addDirectory = React.useCallback(() => {
    const candidate = directoryDraft.trim();
    if (!candidate || !background) return;
    if (background.directories.includes(candidate)) {
      setDirectoryDraft("");
      return;
    }
    void persist(
      { directories: [...background.directories, candidate] },
      `Added ${candidate}`,
    );
    setDirectoryDraft("");
  }, [background, directoryDraft, persist]);

  const removeDirectory = React.useCallback(
    (directory: string) => {
      if (!background) return;
      void persist(
        { directories: background.directories.filter((entry) => entry !== directory) },
        `Removed ${directory}`,
      );
    },
    [background, persist],
  );

  return (
    <div className="sr-pane" data-testid="settings">
      <div className="sr-toolbar">
        <h2 className="sr-section__title">Settings</h2>
      </div>

      {error && (
        <p className="sr-banner sr-banner--error" role="alert" data-testid="settings-error">
          {error}
        </p>
      )}
      {saved && (
        <p className="sr-banner sr-banner--notice" role="status" data-testid="settings-saved">
          {saved}
        </p>
      )}

      <section className="sr-section" data-testid="settings-app">
        <h3 className="sr-section__title">Application</h3>
        <div className="sr-field">
          <label className="sr-field__label" htmlFor="setting-debug">
            <input
              id="setting-debug"
              type="checkbox"
              checked={configBool(config, "debug")}
              disabled={busy}
              onChange={(event) => void toggleDebug(event.target.checked)}
            />{" "}
            Verbose debug logging
          </label>
        </div>
        <div className="sr-field">
          <label className="sr-field__label" htmlFor="setting-remote-lookup">
            <input
              id="setting-remote-lookup"
              type="checkbox"
              checked={configBool(config, "remoteLookupEnabled")}
              disabled={busy}
              onChange={(event) =>
                void persistConfig(
                  "remoteLookupEnabled",
                  event.target.checked,
                  "Remote lookup updated",
                )
              }
            />{" "}
            Allow online metadata lookups
          </label>
        </div>
        <div className="sr-field">
          <label className="sr-field__label" htmlFor="setting-discogs">
            <input
              id="setting-discogs"
              type="checkbox"
              checked={configBool(config, "discogsEnabled")}
              disabled={busy}
              onChange={(event) =>
                void persistConfig("discogsEnabled", event.target.checked, "Discogs updated")
              }
            />{" "}
            Use Discogs
          </label>
        </div>
        <div className="sr-field">
          <label className="sr-field__label" htmlFor="setting-lyrics">
            <input
              id="setting-lyrics"
              type="checkbox"
              checked={configBool(config, "lyricsDownloadEnabled")}
              disabled={busy}
              onChange={(event) =>
                void persistConfig(
                  "lyricsDownloadEnabled",
                  event.target.checked,
                  "Lyrics downloads updated",
                )
              }
            />{" "}
            Download remote lyrics when no local lyrics exist
          </label>
        </div>
        <div className="sr-field">
          <label className="sr-field__label" htmlFor="setting-chinese">
            Chinese script conversion
          </label>
          <select
            id="setting-chinese"
            className="sr-input"
            value={configString(config, "chineseScript")}
            disabled={busy}
            onChange={(event) =>
              void persistConfig(
                "chineseScript",
                event.target.value === "" ? null : event.target.value,
                "Chinese script updated",
              )
            }
          >
            {CHINESE_SCRIPT_OPTIONS.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </div>

        <div className="sr-field">
          <label className="sr-field__label" htmlFor="setting-lyrics-url">
            Lyrics service URL
          </label>
          <div className="sr-toolbar">
            <input
              id="setting-lyrics-url"
              className="sr-input"
              value={lyricsUrlDraft}
              disabled={busy}
              placeholder="https://lrclib.net"
              onChange={(event) => setLyricsUrlDraft(event.target.value)}
            />
            <button
              type="button"
              className="sr-btn"
              disabled={busy}
              onClick={() =>
                void persistConfig("lyricsApiUrl", lyricsUrlDraft.trim(), "Lyrics URL saved")
              }
            >
              Save URL
            </button>
          </div>
        </div>

        <div className="sr-field">
          <label className="sr-field__label" htmlFor="setting-assistant">
            <input
              id="setting-assistant"
              type="checkbox"
              checked={configBool(config, "assistantAutonomous")}
              disabled={busy}
              onChange={(event) =>
                void persistConfig(
                  "assistantAutonomous",
                  event.target.checked,
                  "Assistant behaviour updated",
                )
              }
            />{" "}
            Let the assistant apply its own actions without confirmation
          </label>
        </div>

        <div className="sr-technical" data-testid="settings-runtime">
          <Row label="Version" value={appInfo?.version ?? "—"} />
          <Row label="Runtime" value={appInfo?.runtime ?? "—"} />
          <Row label="Identifier" value={appInfo?.identifier ?? "—"} />
        </div>
      </section>

      <section className="sr-section" data-testid="settings-background">
        <h3 className="sr-section__title">Background tagging</h3>
        <p className="sr-field__static">
          Tagging runs unattended on the schedule below. Each run records its
          outcome, visible under Monitoring.
        </p>

        <div className="sr-field">
          <label className="sr-field__label" htmlFor="setting-background-enabled">
            <input
              id="setting-background-enabled"
              type="checkbox"
              checked={background?.enabled ?? false}
              disabled={busy || !background}
              onChange={(event) =>
                void persist(
                  { enabled: event.target.checked },
                  event.target.checked
                    ? "Background tagging enabled"
                    : "Background tagging disabled",
                )
              }
            />{" "}
            Enable background tagging
          </label>
        </div>

        <div className="sr-field">
          <label className="sr-field__label" htmlFor="setting-schedule-preset">
            Schedule
          </label>
          <select
            id="setting-schedule-preset"
            className="sr-input"
            value={preset}
            disabled={busy || !background}
            onChange={(event) => {
              const value = event.target.value;
              setPreset(value);
              if (value === "custom") return;
              setScheduleDraft(value);
              void persist({ schedule: value }, `Schedule set to ${value}`);
            }}
          >
            {SCHEDULE_PRESETS.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
          <p className="sr-field__static" data-testid="settings-schedule-hint">
            {SCHEDULE_PRESETS.find((option) => option.value === preset)?.hint ?? ""}
          </p>
        </div>

        {preset === "custom" && (
          <div className="sr-field">
            <label className="sr-field__label" htmlFor="setting-schedule-custom">
              Crontab expression
            </label>
            <div className="sr-toolbar">
              <input
                id="setting-schedule-custom"
                className="sr-input"
                value={scheduleDraft}
                disabled={busy}
                placeholder="minute hour day month weekday"
                onChange={(event) => setScheduleDraft(event.target.value)}
              />
              <button
                type="button"
                className="sr-btn sr-btn--primary"
                disabled={busy}
                onClick={() => void persist({ schedule: scheduleDraft }, "Schedule saved")}
              >
                Save schedule
              </button>
            </div>
          </div>
        )}

        <div className="sr-field">
          <label className="sr-field__label" htmlFor="setting-watch">
            <input
              id="setting-watch"
              type="checkbox"
              checked={background?.watchEnabled ?? false}
              disabled={busy || !background}
              onChange={(event) =>
                void persist(
                  { watchEnabled: event.target.checked },
                  event.target.checked
                    ? "Watching for new music"
                    : "Stopped watching for new music",
                )
              }
            />{" "}
            Tag music as it appears in the covered folders
          </label>
        </div>

        <div className="sr-field">
          <label className="sr-field__label" htmlFor="setting-skip-unchanged">
            <input
              id="setting-skip-unchanged"
              type="checkbox"
              checked={background?.skipUnchanged ?? true}
              disabled={busy || !background}
              onChange={(event) =>
                void persist(
                  { skipUnchanged: event.target.checked },
                  "Coverage rule updated",
                )
              }
            />{" "}
            Only tag releases with incomplete tags
          </label>
        </div>

        <div className="sr-field">
          <label className="sr-field__label" htmlFor="setting-directory">
            Covered folders
          </label>
          <div className="sr-toolbar">
            <input
              id="setting-directory"
              className="sr-input"
              value={directoryDraft}
              disabled={busy}
              placeholder="/path/to/music"
              onChange={(event) => setDirectoryDraft(event.target.value)}
            />
            <button
              type="button"
              className="sr-btn"
              disabled={busy || directoryDraft.trim() === ""}
              onClick={addDirectory}
            >
              Add folder
            </button>
          </div>
          {background && background.directories.length > 0 ? (
            <ul className="sr-chips" data-testid="settings-directories">
              {background.directories.map((directory) => (
                <li key={directory} className="sr-chip">
                  {directory}
                  <button
                    type="button"
                    className="sr-btn"
                    aria-label={`Remove ${directory}`}
                    disabled={busy}
                    onClick={() => removeDirectory(directory)}
                  >
                    Remove
                  </button>
                </li>
              ))}
            </ul>
          ) : (
            <p className="sr-field__static" data-testid="settings-directories-empty">
              No folder configured — every mounted library is covered.
            </p>
          )}
        </div>
      </section>
    </div>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="sr-technical__row">
      <span className="sr-technical__label">{label}</span>
      <span>{value}</span>
    </div>
  );
}
