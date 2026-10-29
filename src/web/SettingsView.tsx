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
import type { AppInfo, TagSuggestion } from "../shared/desktop-api";
import {
  getBackgroundSettings,
  setBackgroundSettings,
  type BackgroundTaggingSettings,
} from "../shared/web-adapter";
import { TechnicalRow } from "./TechnicalRow";
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
  suggestTags?: (query: string) => Promise<TagSuggestion[]>;
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
const defaultSuggestTags = (query: string) =>
  window.api?.suggestTagNames?.(query) ?? Promise.resolve([]);
const defaultLoadRoots = () => window.api.listLibraryRoots();
const defaultCheckDirectory = (path: string) => window.api.listDirectory(path);

export function SettingsView({
  loadBackground = defaultLoadBackground,
  saveBackground = defaultSaveBackground,
  loadConfig = defaultLoadConfig,
  saveConfig = defaultSaveConfig,
  loadAppInfo = defaultLoadAppInfo,
  setDebugMode = defaultSetDebugMode,
  suggestTags = defaultSuggestTags,
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
  const [clearTagDraft, setClearTagDraft] = React.useState("");
  const [tagSuggestions, setTagSuggestions] = React.useState<TagSuggestion[]>([]);
  const [supportedTags, setSupportedTags] = React.useState<TagSuggestion[]>([]);
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
  const [activeGroup, setActiveGroup] = React.useState(GROUPS[0].id);

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
    void suggestTags("")
      .then((tags) => {
        if (active) setSupportedTags(tags);
      })
      .catch(() => {
        // Autocomplete is additive; a service that predates this command can
        // still render and save the rest of Settings.
      });
    return () => {
      active = false;
    };
  }, [suggestTags]);

  React.useEffect(() => {
    const query = clearTagDraft.trim();
    if (!query) {
      setTagSuggestions([]);
      return undefined;
    }
    let active = true;
    void suggestTags(query)
      .then((tags) => {
        if (active) setTagSuggestions(tags);
      })
      .catch(() => {
        if (active) setTagSuggestions([]);
      });
    return () => {
      active = false;
    };
  }, [clearTagDraft, suggestTags]);

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

  const selectedClearTags = React.useMemo(() => {
    const value = config.clearTags;
    return Array.isArray(value) && value.every((tag): tag is string => typeof tag === "string")
      ? value
      : [];
  }, [config.clearTags]);

  const addClearTag = React.useCallback(
    (suggestion: TagSuggestion) => {
      if (selectedClearTags.includes(suggestion.id)) {
        setClearTagDraft("");
        return;
      }
      void persistConfig(
        "clearTags",
        [...selectedClearTags, suggestion.id],
        `${suggestion.label} will be cleared`,
      );
      setClearTagDraft("");
    },
    [persistConfig, selectedClearTags],
  );

  const removeClearTag = React.useCallback(
    (tagId: string) => {
      const suggestion = supportedTags.find((tag) => tag.id === tagId);
      void persistConfig(
        "clearTags",
        selectedClearTags.filter((tag) => tag !== tagId),
        `${suggestion?.label ?? tagId} will be kept`,
      );
    },
    [persistConfig, selectedClearTags, supportedTags],
  );

  const addTypedClearTag = React.useCallback(() => {
    const query = clearTagDraft.trim().toLowerCase();
    const exact = tagSuggestions.find(
      (tag) => tag.id.toLowerCase() === query || tag.label.toLowerCase() === query,
    );
    if (exact) {
      addClearTag(exact);
    } else if (query) {
      failField("clearTags", "Choose a tag from the supported ID3 / Navidrome suggestions.");
    }
  }, [addClearTag, clearTagDraft, failField, tagSuggestions]);

  const keptTagLabels = React.useMemo(
    () => supportedTags.filter((tag) => !selectedClearTags.includes(tag.id)).map((tag) => tag.label),
    [selectedClearTags, supportedTags],
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
    setActiveGroup(id);
    const target = document.getElementById(`settings-group-${id}`);
    // Optional call: a non-browser render has the element but no scrolling.
    target?.scrollIntoView?.({ block: "start" });
    target?.querySelector<HTMLElement>(".sr-section__title")?.focus({ preventScroll: true });
  };

  return (
    <div className="sr-pane sr-pane--settings" data-testid="settings">
      <div className="sr-toolbar sr-page-header">
        <div className="sr-page-heading">
          <h1 className="sr-section__title">Settings</h1>
          <p className="sr-page-subtitle">
            Most toggles and preset choices save when changed. Lyrics URL and a custom schedule
            keep a draft until you save.
          </p>
        </div>
        <div className="sr-page-actions">
          {dirty && (
            <span className="sr-freshness" data-testid="settings-dirty">
              Unsaved changes
            </span>
          )}
        </div>
      </div>

      <nav className="sr-tabs sr-tabs--settings" aria-label="Settings sections" data-testid="settings-nav">
        {GROUPS.map((group) => (
          <button
            key={group.id}
            type="button"
            className="sr-tab"
            data-testid={`settings-nav-${group.id}`}
            aria-current={activeGroup === group.id ? "location" : undefined}
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

      <section
        className="sr-section"
        id="settings-group-sources"
        aria-labelledby="settings-heading-sources"
        data-testid="settings-sources"
      >
        <h3 id="settings-heading-sources" className="sr-section__title" tabIndex={-1}>
          Metadata sources
        </h3>
        <p className="sr-field__static">Where Soundrobe may look for metadata it cannot read from the files.</p>

        <div className="sr-field sr-field--stacked">
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

        <div className="sr-field sr-field--stacked">
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

      <section
        className="sr-section"
        id="settings-group-writing"
        aria-labelledby="settings-heading-writing"
        data-testid="settings-writing"
      >
        <h3 id="settings-heading-writing" className="sr-section__title" tabIndex={-1}>
          Tag writing
        </h3>
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

        <div className="sr-field sr-field--stacked">
          <label className="sr-field__label" htmlFor="setting-clear-tags-enabled">
            <input
              id="setting-clear-tags-enabled"
              type="checkbox"
              checked={configBool(config, "clearTagsEnabled")}
              disabled={pending.clearTagsEnabled}
              aria-describedby="setting-clear-tags-hint"
              onChange={(event) =>
                void persistConfig(
                  "clearTagsEnabled",
                  event.target.checked,
                  event.target.checked ? "Tag cleanup enabled" : "Tag cleanup disabled",
                )
              }
            />{" "}
            Clear selected tags before writing metadata
          </label>
          <p className="sr-field__static" id="setting-clear-tags-hint">
            Only the supported ID3 / Navidrome tag catalog can be selected. The cleanup is applied
            in the same write as the new metadata.
          </p>
          {fieldSaved("clearTagsEnabled")}
          {fieldError("clearTagsEnabled")}
        </div>

        {configBool(config, "clearTagsEnabled") && (
          <div className="sr-field sr-field--stacked" data-testid="settings-clear-tags">
            <label className="sr-field__label" htmlFor="setting-clear-tag">
              Selected tags will be cleared
            </label>
            <div className="sr-toolbar">
              <input
                id="setting-clear-tag"
                className="sr-input"
                value={clearTagDraft}
                placeholder="Type Comment or Description"
                role="combobox"
                aria-autocomplete="list"
                aria-controls="setting-clear-tag-suggestions"
                onChange={(event) => setClearTagDraft(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    addTypedClearTag();
                  }
                }}
              />
              <button
                type="button"
                className="sr-btn"
                disabled={pending.clearTags || clearTagDraft.trim() === ""}
                onClick={addTypedClearTag}
              >
                Add tag
              </button>
            </div>
            {tagSuggestions.length > 0 && (
              <div id="setting-clear-tag-suggestions" role="listbox" className="sr-chips">
                {tagSuggestions.map((suggestion) => (
                  <button
                    key={suggestion.id}
                    type="button"
                    role="option"
                    aria-selected={selectedClearTags.includes(suggestion.id)}
                    className="sr-chip sr-btn"
                    disabled={selectedClearTags.includes(suggestion.id) || pending.clearTags}
                    onClick={() => addClearTag(suggestion)}
                    title={`${suggestion.frame} · ${suggestion.source}`}
                  >
                    {suggestion.label}
                  </button>
                ))}
              </div>
            )}
            {selectedClearTags.length > 0 && (
              <ul className="sr-chips" data-testid="settings-clear-tag-list">
                {selectedClearTags.map((tagId) => {
                  const suggestion = supportedTags.find((tag) => tag.id === tagId);
                  return (
                    <li key={tagId} className="sr-chip">
                      {suggestion?.label ?? tagId}
                      <button
                        type="button"
                        className="sr-btn sr-chip__remove"
                        aria-label={`Remove ${suggestion?.label ?? tagId} from clear list`}
                        title="Remove from clear list"
                        disabled={pending.clearTags}
                        onClick={() => removeClearTag(tagId)}
                      >
                        <svg aria-hidden="true" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                          <path d="M6 6 18 18M18 6 6 18" />
                        </svg>
                      </button>
                    </li>
                  );
                })}
              </ul>
            )}
            <p className="sr-field__static" data-testid="settings-kept-tags">
              Tags kept during tagging: {keptTagLabels.length > 0 ? keptTagLabels.join(", ") : "none"}.
              Monitoring shows this list again when each run finishes.
            </p>
            {fieldSaved("clearTags")}
            {fieldError("clearTags")}
          </div>
        )}
      </section>

      <section
        className="sr-section"
        id="settings-group-lyrics"
        aria-labelledby="settings-heading-lyrics"
        data-testid="settings-lyrics"
      >
        <h3 id="settings-heading-lyrics" className="sr-section__title" tabIndex={-1}>
          Lyrics
        </h3>
        <div className="sr-field sr-field--stacked">
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

      <section
        className="sr-section"
        id="settings-group-automation"
        aria-labelledby="settings-heading-automation"
        data-testid="settings-automation"
      >
        <h3 id="settings-heading-automation" className="sr-section__title" tabIndex={-1}>
          Automation
        </h3>
        <p className="sr-field__static">
          Tagging runs unattended on the schedule below. Each run records its outcome, visible
          under Monitoring.
        </p>

        <div className="sr-field sr-field--stacked">
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

        <div className="sr-field sr-field--stacked">
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

        <div className="sr-field sr-field--stacked">
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

      <section
        className="sr-section"
        id="settings-group-assistant"
        aria-labelledby="settings-heading-assistant"
        data-testid="settings-assistant"
      >
        <h3 id="settings-heading-assistant" className="sr-section__title" tabIndex={-1}>
          Assistant
        </h3>
        <div className="sr-field sr-field--stacked">
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

      <section
        className="sr-section"
        id="settings-group-system"
        aria-labelledby="settings-heading-system"
        data-testid="settings-system"
      >
        <h3 id="settings-heading-system" className="sr-section__title" tabIndex={-1}>
          System
        </h3>
        <div className="sr-field sr-field--stacked">
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
          <TechnicalRow label="Version" value={appInfo?.version ?? "—"} />
          <TechnicalRow label="Runtime" value={appInfo?.runtime ?? "—"} />
          <TechnicalRow label="Identifier" value={appInfo?.identifier ?? "—"} />
        </div>
      </section>
    </div>
  );
}
