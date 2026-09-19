/**
 * Settings for the browser runtime.
 *
 * Mirrors the reference product's `系统设置` page: an application section and a
 * background-scraping section, with the reference's own schedule presets and a
 * custom-crontab escape hatch. The crontab is validated by the service, so an
 * invalid expression is refused with the reason rather than stored and silently
 * never fired.
 *
 * Two rules shape this page, both learned from what it used to do wrong:
 *
 * 1. **One save model, and you can see it.** A toggle or a select applies
 *    immediately and confirms itself at the field. A free-text value is not a
 *    decision until it is finished, so it gets Save / Cancel and an
 *    unsaved-changes guard — nothing is discarded silently.
 * 2. **A confirmation belongs to a field and expires.** A page-level "saved"
 *    message that never clears reads as confirmation of whatever is on screen
 *    next, which is how a stale banner becomes a lie.
 */

import React from "react";
import type { AppInfo } from "../shared/desktop-api";
import {
  getBackgroundSettings,
  setBackgroundSettings,
  type BackgroundTaggingSettings,
} from "../shared/web-adapter";
import { describeCron } from "./monitoring";

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
  /** Used to tell whether a covered folder can be read at all. */
  loadRoots?: () => Promise<Array<{ name: string; path: string }>>;
  checkDirectory?: (path: string) => Promise<unknown>;
  /** How long a per-field confirmation stays; injected by tests. */
  confirmMs?: number;
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

/** One group per question a reader arrives with, so the page can be navigated. */
const GROUPS: Array<{ id: string; label: string }> = [
  { id: "sources", label: "Metadata sources" },
  { id: "writing", label: "Tag writing" },
  { id: "lyrics", label: "Lyrics" },
  { id: "automation", label: "Automation" },
  { id: "assistant", label: "Assistant" },
  { id: "system", label: "System" },
];

/** How long a per-field confirmation stays before it clears itself. */
const DEFAULT_CONFIRM_MS = 4000;

function configBool(config: Record<string, unknown>, key: string): boolean {
  return config[key] === true;
}

function configString(config: Record<string, unknown>, key: string): string {
  const value = config[key];
  return typeof value === "string" ? value : "";
}

/**
 * Whether a value looks like a service URL.
 *
 * Deterministic, so it is checked here rather than round-tripped: the service
 * would reject it anyway, and a round trip to learn a syntax error is worse UX.
 */
function urlProblem(value: string): string | null {
  const trimmed = value.trim();
  if (trimmed === "") return null;
  let parsed: URL;
  try {
    parsed = new URL(trimmed);
  } catch {
    return "Enter a full URL, including https://";
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
    return "The URL must start with http:// or https://";
  }
  return null;
}

/**
 * Defaults are module-level constants, never inline expressions here.
 *
 * A default written as `loadRoots = () => window.api.listRoots()` is a *new*
 * function on every render, so an effect depending on it re-runs forever and
 * locks the page. Injected props are stable; defaults have to be made stable.
 */
const defaultLoadBackground = () => getBackgroundSettings();
const defaultSaveBackground = (patch: Partial<BackgroundTaggingSettings>) =>
  setBackgroundSettings(patch);
const defaultLoadConfig = () => window.api.getConfig();
const defaultSaveConfig = (key: string, value: unknown) => window.api.setConfig(key, value);
const defaultLoadAppInfo = () => window.api.appInfo();
const defaultSetDebugMode = (enabled: boolean) => window.api.setDebugMode(enabled);
const defaultLoadRoots = () => window.api.listLibraryRoots();
const defaultCheckDirectory = (path: string) => window.api.listDirectory(path);

export function SettingsView({
  loadBackground = defaultLoadBackground,
  saveBackground = defaultSaveBackground,
  loadConfig = defaultLoadConfig,
  saveConfig = defaultSaveConfig,
  loadAppInfo = defaultLoadAppInfo,
  setDebugMode = defaultSetDebugMode,
  loadRoots = defaultLoadRoots,
  checkDirectory = defaultCheckDirectory,
  confirmMs = DEFAULT_CONFIRM_MS,
}: SettingsViewProps) {
  const [background, setBackground] = React.useState<BackgroundTaggingSettings | null>(null);
  const [scheduleDraft, setScheduleDraft] = React.useState("0 0 * * *");
  const [preset, setPreset] = React.useState("0 0 * * *");
  const [directoryDraft, setDirectoryDraft] = React.useState("");
  const [lyricsUrlDraft, setLyricsUrlDraft] = React.useState("");
  const [storedLyricsUrl, setStoredLyricsUrl] = React.useState("");
  const [config, setConfig] = React.useState<Record<string, unknown>>({});
  const [appInfo, setAppInfo] = React.useState<AppInfo | null>(null);
  const [roots, setRoots] = React.useState<Array<{ name: string; path: string }>>([]);
  const [error, setError] = React.useState<string | null>(null);
  /** Per-field, so a failed save says which field failed. */
  const [fieldErrors, setFieldErrors] = React.useState<Record<string, string>>({});
  /** Per-field confirmations, each of which expires on its own. */
  const [saved, setSaved] = React.useState<Record<string, string>>({});
  /** Per-field pending state, so saving one field does not lock the page. */
  const [pending, setPending] = React.useState<Record<string, boolean>>({});

  const timers = React.useRef<number[]>([]);
  React.useEffect(
    () => () => {
      timers.current.forEach((handle) => window.clearTimeout(handle));
    },
    [],
  );

  /** Confirm at the field that changed, then let the message go. */
  const confirmField = React.useCallback(
    (key: string, message: string) => {
      setSaved((current) => ({ ...current, [key]: message }));
      const handle = window.setTimeout(() => {
        setSaved((current) => {
          const next = { ...current };
          delete next[key];
          return next;
        });
      }, confirmMs);
      timers.current.push(handle);
    },
    [confirmMs],
  );

  const beginField = React.useCallback((key: string) => {
    setPending((current) => ({ ...current, [key]: true }));
    setFieldErrors((current) => {
      if (!(key in current)) return current;
      const next = { ...current };
      delete next[key];
      return next;
    });
  }, []);

  const failField = React.useCallback((key: string, message: string) => {
    setFieldErrors((current) => ({ ...current, [key]: message }));
  }, []);

  const endField = React.useCallback((key: string) => {
    setPending((current) => {
      const next = { ...current };
      delete next[key];
      return next;
    });
  }, []);

  const crontabRef = React.useRef<HTMLInputElement | null>(null);
  const lyricsUrlRef = React.useRef<HTMLInputElement | null>(null);
  const directoryRef = React.useRef<HTMLInputElement | null>(null);

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
        const url = configString(stored, "lyricsApiUrl");
        setLyricsUrlDraft(url);
        setStoredLyricsUrl(url);
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

  /**
   * Which libraries exist, so an added folder can be checked against them.
   *
   * A folder outside every root can never be read, so it would be configured and
   * then silently never tagged. If the roots cannot be read we validate nothing
   * rather than blocking a legitimate change on a failed lookup.
   */
  React.useEffect(() => {
    let active = true;
    void (async () => {
      try {
        const list = await loadRoots();
        if (active) setRoots(list ?? []);
      } catch {
        if (active) setRoots([]);
      }
    })();
    return () => {
      active = false;
    };
  }, [loadRoots]);

  const toggleDebug = React.useCallback(
    async (enabled: boolean) => {
      beginField("debug");
      try {
        await setDebugMode(enabled);
        setConfig((current) => ({ ...current, debug: enabled }));
        confirmField("debug", enabled ? "Debug logging enabled" : "Debug logging disabled");
      } catch (reason) {
        failField("debug", reason instanceof Error ? reason.message : String(reason));
      } finally {
        endField("debug");
      }
    },
    [beginField, confirmField, endField, failField, setDebugMode],
  );

  const persist = React.useCallback(
    async (patch: Partial<BackgroundTaggingSettings>, message: string, field = "background") => {
      beginField(field);
      try {
        const next = await saveBackground(patch);
        setBackground(next);
        setScheduleDraft(next.schedule);
        confirmField(field, message);
        return true;
      } catch (reason) {
        // The service rejects an unusable crontab; surface its wording, at the
        // field that caused it.
        failField(field, reason instanceof Error ? reason.message : String(reason));
        return false;
      } finally {
        endField(field);
      }
    },
    [beginField, confirmField, endField, failField, saveBackground],
  );

  const persistConfig = React.useCallback(
    async (key: string, value: unknown, message: string) => {
      beginField(key);
      try {
        await saveConfig(key, value);
        setConfig((current) => ({ ...current, [key]: value }));
        confirmField(key, message);
      } catch (reason) {
        failField(key, reason instanceof Error ? reason.message : String(reason));
      } finally {
        endField(key);
      }
    },
    [beginField, confirmField, endField, failField, saveConfig],
  );

  const addDirectory = React.useCallback(async () => {
    const candidate = directoryDraft.trim();
    if (!candidate || !background) return;

    // Outside every library root, the service cannot read it, so adding it would
    // configure a folder that silently never gets tagged.
    if (roots.length > 0) {
      const inside = roots.some((root) => {
        const prefix = root.path.endsWith("/") ? root.path : `${root.path}/`;
        return candidate === root.path || candidate.startsWith(prefix);
      });
      if (!inside) {
        failField(
          "directories",
          `Outside the served libraries (${roots.map((root) => root.path).join(", ")}), so it could never be read.`,
        );
        directoryRef.current?.focus();
        return;
      }
    }

    // And an unreadable path is a typo, not a configuration.
    try {
      await checkDirectory(candidate);
    } catch (reason) {
      failField(
        "directories",
        reason instanceof Error ? reason.message : `Cannot read ${candidate}`,
      );
      directoryRef.current?.focus();
      return;
    }

    if (background.directories.includes(candidate)) {
      setDirectoryDraft("");
      return;
    }
    const ok = await persist(
      { directories: [...background.directories, candidate] },
      `Added ${candidate}`,
      "directories",
    );
    if (ok) setDirectoryDraft("");
  }, [background, checkDirectory, directoryDraft, failField, persist, roots]);

  const removeDirectory = React.useCallback(
    (directory: string) => {
      if (!background) return;
      void persist(
        { directories: background.directories.filter((entry) => entry !== directory) },
        `Removed ${directory}`,
        "directories",
      );
    },
    [background, persist],
  );

  const saveLyricsUrl = React.useCallback(async () => {
    const problem = urlProblem(lyricsUrlDraft);
    if (problem) {
      failField("lyricsApiUrl", problem);
      lyricsUrlRef.current?.focus();
      return;
    }
    const value = lyricsUrlDraft.trim();
    await persistConfig("lyricsApiUrl", value, "Lyrics URL saved");
    setStoredLyricsUrl(value);
  }, [failField, lyricsUrlDraft, persistConfig]);

  const saveCustomSchedule = React.useCallback(async () => {
    const ok = await persist({ schedule: scheduleDraft }, "Schedule saved", "schedule");
    if (!ok) crontabRef.current?.focus();
  }, [crontabRef, persist, scheduleDraft]);

  const scheduleWords = describeCron(scheduleDraft);
  const lyricsUrlDirty = lyricsUrlDraft.trim() !== storedLyricsUrl;
  const customScheduleDirty =
    preset === "custom" && background !== null && scheduleDraft !== background.schedule;
  const dirty = lyricsUrlDirty || customScheduleDirty;

  /**
   * Warn before a half-typed value is lost to a page reload. In-page navigation
   * cannot be blocked, which is why the drafts also carry their own visible
   * Save / Cancel state.
   */
  React.useEffect(() => {
    if (!dirty) return undefined;
    const handler = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", handler);
    return () => window.removeEventListener("beforeunload", handler);
  }, [dirty]);

  // One test id per field: several fields can be wrong at once, and a shared id
  // would make "which one failed?" unanswerable.
  const fieldError = (key: string) =>
    fieldErrors[key] ? (
      <p
        className="sr-field__static sr-field__static--error"
        data-testid={`settings-error-${key}`}
        role="alert"
      >
        {fieldErrors[key]}
      </p>
    ) : null;

  const fieldSaved = (key: string) =>
    saved[key] ? (
      <p className="sr-field__static sr-ok" data-testid={`settings-saved-${key}`} role="status">
        {saved[key]}
      </p>
    ) : null;

  const jumpTo = (id: string) => {
    // Optional call: a non-browser render has the element but no scrolling.
    document.getElementById(`settings-group-${id}`)?.scrollIntoView?.({ block: "start" });
  };

  return (
    <div className="sr-pane" data-testid="settings">
      <div className="sr-toolbar">
        <h2 className="sr-section__title">Settings</h2>
        <div className="sr-header__spacer" />
        {dirty && (
          <span className="sr-freshness" data-testid="settings-dirty">
            Unsaved changes
          </span>
        )}
      </div>

      <nav className="sr-tabs" aria-label="Settings sections" data-testid="settings-nav">
        {GROUPS.map((group) => (
          <button
            key={group.id}
            type="button"
            className="sr-tab"
            data-testid={`settings-nav-${group.id}`}
            onClick={() => jumpTo(group.id)}
          >
            {group.label}
          </button>
        ))}
      </nav>

      {error && (
        <p className="sr-banner sr-banner--error" role="alert" data-testid="settings-load-error">
          {error}
        </p>
      )}

      <section className="sr-section" id="settings-group-sources" data-testid="settings-sources">
        <h3 className="sr-section__title">Metadata sources</h3>
        <p className="sr-field__static">Where Soundrobe may look for metadata it cannot read from the files.</p>

        <div className="sr-field">
          <label className="sr-field__label" htmlFor="setting-remote-lookup">
            <input
              id="setting-remote-lookup"
              type="checkbox"
              checked={configBool(config, "remoteLookupEnabled")}
              disabled={pending.remoteLookupEnabled}
              aria-describedby="setting-remote-lookup-hint"
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
          <p className="sr-field__static" id="setting-remote-lookup-hint">
            Turning this off keeps everything local: albums are only tagged from what the files
            already contain and from folder hints.
          </p>
          {fieldSaved("remoteLookupEnabled")}
          {fieldError("remoteLookupEnabled")}
        </div>

        <div className="sr-field">
          <label className="sr-field__label" htmlFor="setting-discogs">
            <input
              id="setting-discogs"
              type="checkbox"
              checked={configBool(config, "discogsEnabled")}
              disabled={pending.discogsEnabled}
              aria-describedby="setting-discogs-hint"
              onChange={(event) =>
                void persistConfig("discogsEnabled", event.target.checked, "Discogs updated")
              }
            />{" "}
            Use Discogs
          </label>
          <p className="sr-field__static" id="setting-discogs-hint">
            A second source, tried after MusicBrainz. Needs a Discogs token configured on the
            service.
          </p>
          {fieldSaved("discogsEnabled")}
          {fieldError("discogsEnabled")}
        </div>
      </section>

      <section className="sr-section" id="settings-group-writing" data-testid="settings-writing">
        <h3 className="sr-section__title">Tag writing</h3>
        <div className="sr-field">
          <label className="sr-field__label" htmlFor="setting-chinese">
            Chinese script conversion
          </label>
          <select
            id="setting-chinese"
            className="sr-input"
            value={configString(config, "chineseScript")}
            disabled={pending.chineseScript}
            aria-describedby="setting-chinese-hint"
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
          <p className="sr-field__static" id="setting-chinese-hint">
            Converts Chinese text in tags as it is written. "Leave unchanged" writes what the file
            already had.
          </p>
          {fieldSaved("chineseScript")}
          {fieldError("chineseScript")}
        </div>
      </section>

      <section className="sr-section" id="settings-group-lyrics" data-testid="settings-lyrics">
        <h3 className="sr-section__title">Lyrics</h3>
        <div className="sr-field">
          <label className="sr-field__label" htmlFor="setting-lyrics">
            <input
              id="setting-lyrics"
              type="checkbox"
              checked={configBool(config, "lyricsDownloadEnabled")}
              disabled={pending.lyricsDownloadEnabled}
              aria-describedby="setting-lyrics-hint"
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
          <p className="sr-field__static" id="setting-lyrics-hint">
            Local lyrics always win. This only fills in albums that have none at all.
          </p>
          {fieldSaved("lyricsDownloadEnabled")}
          {fieldError("lyricsDownloadEnabled")}
        </div>

        <div className="sr-field">
          <label className="sr-field__label" htmlFor="setting-lyrics-url">
            Lyrics service URL
          </label>
          <div className="sr-toolbar">
            <input
              id="setting-lyrics-url"
              ref={lyricsUrlRef}
              className="sr-input"
              value={lyricsUrlDraft}
              disabled={pending.lyricsApiUrl}
              placeholder="https://lrclib.net"
              aria-invalid={fieldErrors.lyricsApiUrl ? true : undefined}
              aria-describedby="setting-lyrics-url-hint"
              onChange={(event) => setLyricsUrlDraft(event.target.value)}
            />
            <button
              type="button"
              className="sr-btn sr-btn--primary"
              disabled={pending.lyricsApiUrl || !lyricsUrlDirty}
              onClick={() => void saveLyricsUrl()}
            >
              Save
            </button>
            <button
              type="button"
              className="sr-btn"
              disabled={pending.lyricsApiUrl || !lyricsUrlDirty}
              onClick={() => setLyricsUrlDraft(storedLyricsUrl)}
            >
              Cancel
            </button>
          </div>
          <p className="sr-field__static" id="setting-lyrics-url-hint">
            The lyrics provider to ask. Stored only when you save it, so a half-typed address is
            never used.
          </p>
          {fieldSaved("lyricsApiUrl")}
          {fieldError("lyricsApiUrl")}
        </div>
      </section>

      <section className="sr-section" id="settings-group-automation" data-testid="settings-automation">
        <h3 className="sr-section__title">Automation</h3>
        <p className="sr-field__static">
          Tagging runs unattended on the schedule below. Each run records its outcome, visible
          under Monitoring.
        </p>

        <div className="sr-field">
          <label className="sr-field__label" htmlFor="setting-background-enabled">
            <input
              id="setting-background-enabled"
              type="checkbox"
              checked={background?.enabled ?? false}
              disabled={pending.background || !background}
              aria-describedby="setting-background-enabled-hint"
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
          <p className="sr-field__static" id="setting-background-enabled-hint">
            Off means nothing is tagged until you start it by hand from Monitoring.
          </p>
          {fieldSaved("background")}
          {fieldError("background")}
        </div>

        <div className="sr-field">
          <label className="sr-field__label" htmlFor="setting-schedule-preset">
            Schedule
          </label>
          <select
            id="setting-schedule-preset"
            className="sr-input"
            value={preset}
            disabled={pending.background || !background}
            aria-describedby="setting-schedule-hint"
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
          <p className="sr-field__static" id="setting-schedule-hint" data-testid="settings-schedule-hint">
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
                ref={crontabRef}
                className="sr-input"
                value={scheduleDraft}
                disabled={pending.schedule}
                placeholder="minute hour day month weekday"
                aria-invalid={fieldErrors.schedule ? true : undefined}
                aria-describedby="setting-schedule-custom-hint"
                onChange={(event) => setScheduleDraft(event.target.value)}
              />
              <button
                type="button"
                className="sr-btn sr-btn--primary"
                disabled={pending.schedule || !customScheduleDirty}
                onClick={() => void saveCustomSchedule()}
              >
                Save schedule
              </button>
              <button
                type="button"
                className="sr-btn"
                disabled={pending.schedule || !customScheduleDirty}
                onClick={() => setScheduleDraft(background?.schedule ?? scheduleDraft)}
              >
                Cancel
              </button>
            </div>
            {/*
              A five-field expression is not readable, so the page says what it
              means. When it cannot be sure, it says that instead of guessing:
              the service owns when this actually fires.
            */}
            <p className="sr-field__static" id="setting-schedule-custom-hint" data-testid="settings-schedule-words">
              {scheduleWords ?? "This pattern is not one the page can put into words — check the five fields."}
            </p>
            {fieldSaved("schedule")}
            {fieldError("schedule")}
          </div>
        )}

        <div className="sr-field">
          <label className="sr-field__label" htmlFor="setting-watch">
            <input
              id="setting-watch"
              type="checkbox"
              checked={background?.watchEnabled ?? false}
              disabled={pending.background || !background}
              aria-describedby="setting-watch-hint"
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
          <p className="sr-field__static" id="setting-watch-hint">
            New albums are picked up shortly after they appear, without waiting for the schedule.
          </p>
        </div>

        <div className="sr-field">
          <label className="sr-field__label" htmlFor="setting-skip-unchanged">
            <input
              id="setting-skip-unchanged"
              type="checkbox"
              checked={background?.skipUnchanged ?? true}
              disabled={pending.background || !background}
              aria-describedby="setting-skip-unchanged-hint"
              onChange={(event) =>
                void persist({ skipUnchanged: event.target.checked }, "Coverage rule updated")
              }
            />{" "}
            Only tag releases with incomplete tags
          </label>
          <p className="sr-field__static" id="setting-skip-unchanged-hint">
            Keeps a run short by leaving already-complete albums alone.
          </p>
        </div>

        <div className="sr-field">
          <label className="sr-field__label" htmlFor="setting-directory">
            Covered folders
          </label>
          <div className="sr-toolbar">
            <input
              id="setting-directory"
              ref={directoryRef}
              className="sr-input"
              value={directoryDraft}
              disabled={pending.directories}
              placeholder="/path/to/music"
              aria-invalid={fieldErrors.directories ? true : undefined}
              aria-describedby="setting-directory-hint"
              onChange={(event) => setDirectoryDraft(event.target.value)}
            />
            <button
              type="button"
              className="sr-btn"
              disabled={pending.directories || directoryDraft.trim() === ""}
              onClick={() => void addDirectory()}
            >
              Add folder
            </button>
          </div>
          <p className="sr-field__static" id="setting-directory-hint">
            {roots.length > 0
              ? `Must be inside a served library: ${roots.map((root) => root.path).join(", ")}.`
              : "Leave empty to cover every mounted library."}
          </p>
          {fieldError("directories")}
          {background && background.directories.length > 0 ? (
            <ul className="sr-chips" data-testid="settings-directories">
              {background.directories.map((directory) => (
                <li key={directory} className="sr-chip">
                  {directory}
                  <button
                    type="button"
                    className="sr-btn"
                    aria-label={`Remove ${directory}`}
                    disabled={pending.directories}
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

      <section className="sr-section" id="settings-group-assistant" data-testid="settings-assistant">
        <h3 className="sr-section__title">Assistant</h3>
        <div className="sr-field">
          <label className="sr-field__label" htmlFor="setting-assistant">
            <input
              id="setting-assistant"
              type="checkbox"
              checked={configBool(config, "assistantAutonomous")}
              disabled={pending.assistantAutonomous}
              aria-describedby="setting-assistant-hint"
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
          <p className="sr-field__static" id="setting-assistant-hint">
            Off by default, and deliberately so: with this on the assistant can write tags
            without asking you first.
          </p>
          {fieldSaved("assistantAutonomous")}
          {fieldError("assistantAutonomous")}
        </div>
      </section>

      <section className="sr-section" id="settings-group-system" data-testid="settings-system">
        <h3 className="sr-section__title">System</h3>
        <div className="sr-field">
          <label className="sr-field__label" htmlFor="setting-debug">
            <input
              id="setting-debug"
              type="checkbox"
              checked={configBool(config, "debug")}
              disabled={pending.debug}
              aria-describedby="setting-debug-hint"
              onChange={(event) => void toggleDebug(event.target.checked)}
            />{" "}
            Verbose debug logging
          </label>
          <p className="sr-field__static" id="setting-debug-hint">
            Writes a per-release trace, readable under Monitoring → Logs. Useful when a run
            leaves something flagged.
          </p>
          {fieldSaved("debug")}
          {fieldError("debug")}
        </div>

        <div className="sr-technical" data-testid="settings-runtime">
          <Row label="Version" value={appInfo?.version ?? "—"} />
          <Row label="Runtime" value={appInfo?.runtime ?? "—"} />
          <Row label="Identifier" value={appInfo?.identifier ?? "—"} />
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
