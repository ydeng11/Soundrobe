/**
 * Monitoring for the browser runtime.
 *
 * The reference product's monitoring surfaces are a service-log viewer with
 * selectable sources plus background task state. Soundrobe has no Celery or
 * Redis, so this view maps those sources onto the things this app genuinely
 * writes: its own application log, its verbose auto-tag trace, the scheduler
 * state, and the task registry.
 *
 * Everything rendered here is real backend state — the schedule that will
 * actually fire, the runs that actually happened, and the releases the last
 * survey actually classified. Nothing is inferred in the browser.
 *
 * Shape: an always-visible strip of counts, over four peer tabs. The strip is
 * the overview and every tile in it is a way in; the tabs are the detail, one
 * question each. That is the reference's own structure for the same problem
 * (`操作日志` / `运行日志` over a faceted list), and it is why the sections are
 * tabs rather than a six-section scroll.
 */

import React from "react";
import { OperationLogView } from "./OperationLogView";
import { TechnicalRow } from "./TechnicalRow";
import { albumNameOf } from "./review";
import {
  LOG_LEVEL_LABEL,
  LOG_LEVEL_ORDER,
  RELEASE_CLASS,
  RELEASE_LABEL,
  RUN_STATUS_CLASS,
  RUN_STATUS_LABEL,
  TRIGGER_LABEL,
  describeCron,
  describeRunItemReason,
  describeRunItems,
  facetCounts,
  filterLog,
  filterReleases,
  formatRunDuration,
  itemsNeedingAction,
  levelCounts,
  parseLog,
  relativeTime,
  type LogLevel,
} from "./monitoring";
import {
  getMonitoringStatus,
  listMonitoringReleases,
  readMonitoringLog,
  retryFailedBackgroundTagging,
  runBackgroundTagging,
  type BackgroundRun,
  type BackgroundRunTrigger,
  type MonitoredRelease,
  type MonitoringLogSource,
  type MonitoringStatus,
  type ReleaseHealthStatus,
  type WebTask,
} from "../shared/web-adapter";
import type { MonitoringLog } from "../shared/web-adapter";

export interface MonitoringViewProps {
  /** Injected for tests. */
  loadStatus?: () => Promise<MonitoringStatus>;
  loadReleases?: () => Promise<{ releases: MonitoredRelease[] }>;
  readLog?: (source: MonitoringLogSource, lines?: number) => Promise<MonitoringLog>;
  runNow?: () => Promise<{ taskId: string }>;
  /**
   * Re-run the releases a finished run left needing action. Monitoring is where
   * the failures are visible, so it is where they can be acted on.
   */
  retryFailed?: (runId: string) => Promise<{ taskId: string }>;
  /** The operation log is a tab of this view rather than a destination. */
  loadTasks?: () => Promise<WebTask[]>;
  cancelTask?: (taskId: string) => Promise<unknown>;
  /**
   * Opens an album in the Workspace. Monitoring's job is to say what needs
   * attention; without this it could only describe the problem.
   */
  onOpenAlbum?: (path: string) => void;
  onOpenSettings?: () => void;
  /** Poll interval; 0 disables polling (used by tests). */
  refreshMs?: number;
  /** Countdown tick; 0 disables it (used by tests). */
  tickMs?: number;
}

type MonitoringTab = "overview" | "releases" | "operations" | "logs";

const TABS: Array<{ id: MonitoringTab; label: string }> = [
  { id: "overview", label: "Overview" },
  { id: "releases", label: "Releases" },
  { id: "operations", label: "Operations" },
  { id: "logs", label: "Logs" },
];

/** How many release rows are rendered before "Show more". */
const RELEASE_PAGE = 50;
/** Log lines fetched at once. */
const LOG_PAGE = 500;

const LOG_SOURCES: Array<{ value: MonitoringLogSource; label: string }> = [
  { value: "app", label: "Application log" },
  { value: "auto-tag", label: "Auto-tag trace" },
];

/** Local wall-clock rendering of an RFC3339 timestamp from the service. */
function formatStamp(value: string | null): string {
  if (!value) return "—";
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? value : parsed.toLocaleString();
}

/** The zone this browser is rendering in, so a schedule's clock is never a guess. */
function browserZone(): string {
  try {
    const parts = new Intl.DateTimeFormat(undefined, { timeZoneName: "short" }).formatToParts(
      new Date(),
    );
    return parts.find((part) => part.type === "timeZoneName")?.value ?? "";
  } catch {
    return "";
  }
}

/**
 * Defaults are module-level constants, never inline expressions here.
 *
 * A default written inline is a *new* function on every render, so any effect
 * that depends on it re-runs forever. Injected props are stable; defaults have
 * to be made stable.
 */
const defaultLoadStatus = () => getMonitoringStatus();
const defaultLoadReleases = () => listMonitoringReleases();
const defaultReadLog = (source: MonitoringLogSource, lines?: number) => readMonitoringLog(source, lines);
const defaultRunNow = () => runBackgroundTagging();
const defaultRetryFailed = (runId: string) => retryFailedBackgroundTagging(runId);
const defaultLoadTasks = () => import("../shared/web-adapter").then((m) => m.listWebTasks());
const defaultCancelTask = (taskId: string) => window.api.cancelTask(taskId);

export function MonitoringView({
  loadStatus = defaultLoadStatus,
  loadReleases = defaultLoadReleases,
  readLog = defaultReadLog,
  runNow = defaultRunNow,
  retryFailed = defaultRetryFailed,
  loadTasks = defaultLoadTasks,
  cancelTask = defaultCancelTask,
  onOpenAlbum,
  onOpenSettings,
  refreshMs = 5000,
  tickMs = 1000,
}: MonitoringViewProps) {
  const [status, setStatus] = React.useState<MonitoringStatus | null>(null);
  const [releases, setReleases] = React.useState<MonitoredRelease[]>([]);
  const [log, setLog] = React.useState<MonitoringLog | null>(null);
  const [source, setSource] = React.useState<MonitoringLogSource>("app");
  const [error, setError] = React.useState<string | null>(null);
  const [notice, setNotice] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState(false);
  const [tab, setTab] = React.useState<MonitoringTab>("overview");
  const [releaseFilter, setReleaseFilter] = React.useState<ReleaseHealthStatus | "all">("all");
  const [releaseQuery, setReleaseQuery] = React.useState("");
  const [releaseLimit, setReleaseLimit] = React.useState(RELEASE_PAGE);
  const [expandedRun, setExpandedRun] = React.useState<string | null>(null);
  const [logLevel, setLogLevel] = React.useState<LogLevel | "all">("all");
  const [logQuery, setLogQuery] = React.useState("");
  const [lastUpdated, setLastUpdated] = React.useState<Date | null>(null);
  const [clock, setClock] = React.useState(() => new Date());

  const refresh = React.useCallback(async () => {
    try {
      const [next, monitored] = await Promise.all([loadStatus(), loadReleases()]);
      setStatus(next);
      setReleases(monitored.releases);
      setError(null);
      setLastUpdated(new Date());
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  }, [loadStatus, loadReleases]);

  React.useEffect(() => {
    void refresh();
  }, [refresh]);

  /**
   * Poll only while the tab is actually being looked at. A background tab
   * polling a library-sized payload every five seconds is waste nobody sees.
   */
  const tabIsHidden = React.useCallback(
    () => typeof document !== "undefined" && document.visibilityState === "hidden",
    [],
  );

  React.useEffect(() => {
    if (refreshMs <= 0) return undefined;
    const timer = window.setInterval(() => {
      if (tabIsHidden()) return;
      void refresh();
    }, refreshMs);
    return () => window.clearInterval(timer);
  }, [refresh, refreshMs, tabIsHidden]);

  /** Drives the countdown words; without it "in 45s" would sit frozen. */
  React.useEffect(() => {
    if (tickMs <= 0) return undefined;
    const timer = window.setInterval(() => setClock(new Date()), tickMs);
    return () => window.clearInterval(timer);
  }, [tickMs]);

  const loadLog = React.useCallback(
    async (nextSource: MonitoringLogSource, lines: number) => {
      try {
        const next = await readLog(nextSource, lines);
        setLog(next);
        setNotice(null);
      } catch (reason) {
        // A source that has never written a file is an ordinary state, not a
        // failure of the page.
        setLog(null);
        setNotice(reason instanceof Error ? reason.message : String(reason));
      }
    },
    [readLog],
  );

  React.useEffect(() => {
    void loadLog(source, LOG_PAGE);
  }, [loadLog, source]);

  /**
   * Keep the log live. It used to be read once per source change, so a run in
   * progress never updated the very output you opened the page to watch.
   */
  const logLive = tab === "logs" && refreshMs > 0 && logLevel === "all" && logQuery === "";
  React.useEffect(() => {
    if (!logLive) return undefined;
    const timer = window.setInterval(() => {
      if (tabIsHidden()) return;
      void loadLog(source, LOG_PAGE);
    }, refreshMs);
    return () => window.clearInterval(timer);
  }, [loadLog, source, logLive, refreshMs, tabIsHidden]);

  const handleRunNow = React.useCallback(async () => {
    setBusy(true);
    setNotice(null);
    try {
      const { taskId } = await runNow();
      setNotice(`Background tagging started (${taskId}).`);
      await refresh();
    } catch (reason) {
      // A refused start (disabled, busy, nothing to cover) is reported verbatim.
      setNotice(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  }, [refresh, runNow]);

  const handleRetryFailed = React.useCallback(
    async (runId: string) => {
      setBusy(true);
      setNotice(null);
      try {
        const { taskId } = await retryFailed(runId);
        setNotice(`Retrying the releases that failed (${taskId}).`);
        await refresh();
      } catch (reason) {
        setNotice(reason instanceof Error ? reason.message : String(reason));
      } finally {
        setBusy(false);
      }
    },
    [refresh, retryFailed],
  );

  const runs = status?.recentRuns ?? [];
  const running = status?.running ?? [];
  const facets = React.useMemo(() => facetCounts(releases), [releases]);
  const facetTotal = releases.length;

  const shownReleases = React.useMemo(
    () => filterReleases(releases, releaseFilter, releaseQuery),
    [releases, releaseFilter, releaseQuery],
  );
  const visibleReleases = shownReleases.slice(0, releaseLimit);

  const parsedLog = React.useMemo(() => parseLog(log?.lines ?? []), [log]);
  const counts = React.useMemo(() => levelCounts(parsedLog), [parsedLog]);
  const shownLog = React.useMemo(
    () => filterLog(parsedLog, logLevel, logQuery),
    [parsedLog, logLevel, logQuery],
  );
  const filtersActive = releaseFilter !== "all" || releaseQuery.trim() !== "";
  const zone = React.useMemo(browserZone, []);

  /** A tile that opens the list it counts. */
  const openFacet = (status: ReleaseHealthStatus) => {
    setReleaseFilter(status);
    setReleaseQuery("");
    setReleaseLimit(RELEASE_PAGE);
    setTab("releases");
  };

  const scheduleWords = status ? describeCron(status.schedule) : null;

  return (
    <div className="sr-pane" data-testid="monitoring">
      <div className="sr-toolbar">
        <h2 className="sr-section__title">Monitoring</h2>
        <div className="sr-header__spacer" />
        {lastUpdated && (
          <span className="sr-freshness" data-testid="monitoring-freshness">
            Updated {relativeTime(lastUpdated.toISOString(), clock)}
          </span>
        )}
        <button
          type="button"
          className="sr-btn sr-btn--primary"
          disabled={busy || status?.enabled === false}
          onClick={() => void handleRunNow()}
        >
          Tag now
        </button>
        <button type="button" className="sr-btn" onClick={() => void refresh()}>
          Refresh
        </button>
      </div>

      {/*
        The overview strip. Every count that has a list behind it is a button, so
        "Needs review 5" is the way to those five rather than a number to note.
      */}
      <div className="sr-summary" data-testid="monitoring-summary">
        <Tile
          testId="monitoring-tile-enabled"
          label="Background tagging"
          value={status?.enabled ? "On" : "Off"}
        />
        <Tile
          testId="monitoring-tile-next-run"
          label="Next run"
          value={status?.nextRunAt ? relativeTime(status.nextRunAt, clock) : "—"}
          hint={status?.nextRunAt ? formatStamp(status.nextRunAt) : undefined}
        />
        {running.length > 0 && (
          <Tile
            testId="monitoring-tile-running"
            label="Running now"
            value={running.length}
            onClick={() => setTab("operations")}
          />
        )}
        {facets.map((facet) => (
          <Tile
            key={facet.status}
            testId={`monitoring-tile-${facet.status}`}
            label={facet.label}
            value={facet.count}
            onClick={() => openFacet(facet.status)}
            active={tab === "releases" && releaseFilter === facet.status}
          />
        ))}
        {facetTotal === 0 && (
          <Tile testId="monitoring-tile-total" label="Releases surveyed" value={0} />
        )}
      </div>

      {/* Announce completion for anyone not watching the numbers. */}
      <p className="sr-sr-only" role="status" aria-live="polite" data-testid="monitoring-live">
        {running.length > 0
          ? `${running.length} background run in progress.`
          : "No background run in progress."}
      </p>

      {error && (
        <p className="sr-banner sr-banner--error" role="alert">
          {error}
        </p>
      )}
      {notice && (
        <p className="sr-banner sr-banner--notice" role="status" data-testid="monitoring-notice">
          {notice}
        </p>
      )}

      <div className="sr-tabs" role="tablist" aria-label="Monitoring views">
        {TABS.map((entry) => (
          <button
            key={entry.id}
            type="button"
            role="tab"
            id={`monitoring-tab-${entry.id}`}
            aria-selected={tab === entry.id}
            aria-controls={`monitoring-panel-${entry.id}`}
            className={`sr-tab${tab === entry.id ? " is-active" : ""}`}
            data-testid={`monitoring-tab-${entry.id}`}
            onClick={() => setTab(entry.id)}
          >
            {entry.label}
            {entry.id === "releases" && filtersActive && (
              <span className="sr-tab__count">{shownReleases.length}</span>
            )}
          </button>
        ))}
      </div>

      {tab === "overview" && (
        <div role="tabpanel" id="monitoring-panel-overview" aria-labelledby="monitoring-tab-overview">
          <section className="sr-section" data-testid="monitoring-schedule">
            <div className="sr-toolbar">
              <h3 className="sr-section__title">Schedule</h3>
              <div className="sr-header__spacer" />
              {onOpenSettings && (
                <button type="button" className="sr-btn" onClick={onOpenSettings}>
                  Edit in Settings
                </button>
              )}
            </div>
            {status ? (
              <>
                <div className="sr-technical">
                  <TechnicalRow label="State" value={status.enabled ? "Enabled" : "Disabled"} />
                  <TechnicalRow label="Crontab" value={status.schedule} mono />
                  <TechnicalRow
                    label="Means"
                    value={scheduleWords ?? "Not a pattern this page can put into words"}
                  />
                  <TechnicalRow
                    label="Next run"
                    value={
                      status.nextRunAt
                        ? `${relativeTime(status.nextRunAt, clock)} — ${formatStamp(status.nextRunAt)}`
                        : "—"
                    }
                  />
                  <TechnicalRow label="Watch new files" value={status.watchEnabled ? "Yes" : "No"} />
                  <TechnicalRow label="Skip fully tagged" value={status.skipUnchanged ? "Yes" : "No"} />
                  <TechnicalRow
                    label="Coverage"
                    value={
                      status.effectiveDirectories.length > 0
                        ? status.effectiveDirectories.join(", ")
                        : "No directory is available"
                    }
                  />
                </div>
                <p className="sr-field__static" data-testid="monitoring-timezone">
                  Times are shown in this browser’s timezone{zone ? ` (${zone})` : ""}. The
                  schedule runs in the service’s local time.
                </p>
              </>
            ) : (
              <p className="sr-empty__body">Loading schedule…</p>
            )}
          </section>

          <section className="sr-section" data-testid="monitoring-running">
            <h3 className="sr-section__title">Running now</h3>
            {running.length === 0 ? (
              <p className="sr-empty__body">No background run is in progress.</p>
            ) : (
              <div className="sr-table-wrap">
                <table className="sr-table sr-table--log">
                  <thead>
                    <tr>
                      <th scope="col">Trigger</th>
                      <th scope="col">Progress</th>
                      <th scope="col">Step</th>
                    </tr>
                  </thead>
                  <tbody>
                    {running.map((task) => (
                      <tr key={task.taskId}>
                        <td>{triggerOf(task, runs)}</td>
                        <td>
                          <Progress task={task} />
                        </td>
                        <td className="sr-cell--muted">{task.message || "—"}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </section>

          <section className="sr-section" data-testid="monitoring-runs">
            <div className="sr-toolbar">
              <h3 className="sr-section__title">Recent runs</h3>
              <div className="sr-header__spacer" />
              <span className="sr-freshness">last {runs.length} kept</span>
            </div>
            {runs.length === 0 ? (
              <p className="sr-empty__body" data-testid="monitoring-runs-empty">
                No background run has been recorded yet.
              </p>
            ) : (
              <div className="sr-table-wrap">
                <table className="sr-table sr-table--log" data-testid="monitoring-runs-table">
                  <thead>
                    <tr>
                      <th scope="col">Started</th>
                      <th scope="col">Trigger</th>
                      <th scope="col">Status</th>
                      <th scope="col">Took</th>
                      <th scope="col">Tagged</th>
                      <th scope="col">Failed</th>
                      <th scope="col">Releases</th>
                      <th scope="col">Summary</th>
                    </tr>
                  </thead>
                  <tbody>
                    {runs.map((run) => (
                      <React.Fragment key={run.id}>
                        <tr>
                          <td title={formatStamp(run.startedAt)}>
                            {relativeTime(run.startedAt, clock)}
                          </td>
                          <td>{TRIGGER_LABEL[run.trigger] ?? run.trigger}</td>
                          <td>
                            <span className={`sr-badge ${RUN_STATUS_CLASS[run.status]}`}>
                              {RUN_STATUS_LABEL[run.status] ?? run.status}
                            </span>
                          </td>
                          <td>{formatRunDuration(run.startedAt, run.finishedAt, clock)}</td>
                          <td>{run.tagged}</td>
                          <td>{run.failed}</td>
                          <td>
                            {run.items.length === 0 ? (
                              <span className="sr-cell--muted">—</span>
                            ) : (
                              <button
                                type="button"
                                className="sr-btn"
                                aria-expanded={expandedRun === run.id}
                                aria-label={`Show the ${run.items.length} releases this run recorded`}
                                data-testid={`monitoring-run-items-${run.id}`}
                                onClick={() =>
                                  setExpandedRun(expandedRun === run.id ? null : run.id)
                                }
                              >
                                {run.items.length}
                              </button>
                            )}
                          </td>
                          <td className="sr-cell--muted">{run.message || "—"}</td>
                        </tr>
                        {expandedRun === run.id && (
                          <tr data-testid={`monitoring-run-detail-${run.id}`}>
                            <td colSpan={8}>
                              <RunLedger
                                run={run}
                                busy={busy}
                                onRetry={handleRetryFailed}
                                onOpenAlbum={onOpenAlbum}
                              />
                            </td>
                          </tr>
                        )}
                      </React.Fragment>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </section>
        </div>
      )}

      {tab === "releases" && (
        <div role="tabpanel" id="monitoring-panel-releases" aria-labelledby="monitoring-tab-releases">
          <section className="sr-section" data-testid="monitoring-releases">
            <div className="sr-filterbar">
              <div className="sr-facets" role="group" aria-label="Filter by tagging state">
                <button
                  type="button"
                  className={`sr-facet${releaseFilter === "all" ? " is-active" : ""}`}
                  aria-pressed={releaseFilter === "all"}
                  data-testid="release-filter-all"
                  onClick={() => {
                    setReleaseFilter("all");
                    setReleaseLimit(RELEASE_PAGE);
                  }}
                >
                  All <span className="sr-facet__count">{facetTotal}</span>
                </button>
                {facets.map((facet) => (
                  <button
                    key={facet.status}
                    type="button"
                    className={`sr-facet${releaseFilter === facet.status ? " is-active" : ""}`}
                    aria-pressed={releaseFilter === facet.status}
                    data-testid={`release-filter-${facet.status}`}
                    onClick={() => {
                      setReleaseFilter(facet.status);
                      setReleaseLimit(RELEASE_PAGE);
                    }}
                  >
                    {facet.label} <span className="sr-facet__count">{facet.count}</span>
                  </button>
                ))}
              </div>
              <label className="sr-sr-only" htmlFor="release-search">
                Search releases by path
              </label>
              <input
                id="release-search"
                className="sr-input"
                type="search"
                placeholder="Search by path…"
                value={releaseQuery}
                data-testid="release-search"
                onChange={(event) => {
                  setReleaseQuery(event.target.value);
                  setReleaseLimit(RELEASE_PAGE);
                }}
              />
              {filtersActive && (
                <button
                  type="button"
                  className="sr-btn"
                  data-testid="release-clear"
                  onClick={() => {
                    setReleaseFilter("all");
                    setReleaseQuery("");
                    setReleaseLimit(RELEASE_PAGE);
                  }}
                >
                  Clear filters
                </button>
              )}
            </div>

            <p className="sr-field__static" data-testid="release-count">
              Showing {visibleReleases.length} of {shownReleases.length}
              {filtersActive ? ` matching (${facetTotal} surveyed)` : " releases"}
            </p>

            {facetTotal === 0 ? (
              <p className="sr-empty__body">
                No release has been surveyed yet. The first background run or startup survey
                will fill this list.
              </p>
            ) : shownReleases.length === 0 ? (
              <p className="sr-empty__body" data-testid="release-empty-filtered">
                No release matches these filters.
              </p>
            ) : (
              <>
                <div className="sr-table-wrap">
                  <table className="sr-table sr-table--log">
                    <thead>
                      <tr>
                        <th scope="col">Release</th>
                        <th scope="col">Tagging state</th>
                        <th scope="col">Path</th>
                        {onOpenAlbum && <th scope="col">Actions</th>}
                      </tr>
                    </thead>
                    <tbody>
                      {visibleReleases.map((release) => (
                        <tr key={release.path}>
                          <td>
                            <span
                              role={onOpenAlbum ? "button" : undefined}
                              tabIndex={onOpenAlbum ? 0 : undefined}
                              className="sr-cell--primary"
                              onClick={() => onOpenAlbum?.(release.path)}
                              onKeyDown={(event) => {
                                if (!onOpenAlbum) return;
                                if (event.key === "Enter" || event.key === " ") {
                                  event.preventDefault();
                                  onOpenAlbum(release.path);
                                }
                              }}
                            >
                              {albumNameOf(release.path)}
                            </span>
                          </td>
                          <td>
                            <span className={`sr-badge ${RELEASE_CLASS[release.status]}`}>
                              {RELEASE_LABEL[release.status] ?? release.status}
                            </span>
                          </td>
                          <td className="sr-cell--muted">{release.path}</td>
                          {onOpenAlbum && (
                            <td>
                              <button
                                type="button"
                                className="sr-btn"
                                data-testid={`release-open-${release.path}`}
                                onClick={() => onOpenAlbum(release.path)}
                              >
                                Open
                              </button>
                            </td>
                          )}
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
                {shownReleases.length > visibleReleases.length && (
                  <button
                    type="button"
                    className="sr-btn"
                    data-testid="release-show-more"
                    onClick={() => setReleaseLimit((current) => current + RELEASE_PAGE)}
                  >
                    Show {Math.min(RELEASE_PAGE, shownReleases.length - visibleReleases.length)}{" "}
                    more
                  </button>
                )}
              </>
            )}
          </section>
        </div>
      )}

      {tab === "operations" && (
        <div
          role="tabpanel"
          id="monitoring-panel-operations"
          aria-labelledby="monitoring-tab-operations"
        >
          <OperationLogView loadTasks={loadTasks} cancelTask={cancelTask} />
        </div>
      )}

      {tab === "logs" && (
        <div role="tabpanel" id="monitoring-panel-logs" aria-labelledby="monitoring-tab-logs">
          <section className="sr-section" data-testid="monitoring-logs">
            <div className="sr-toolbar">
              <h3 className="sr-section__title">Service logs</h3>
              <div className="sr-header__spacer" />
              <label className="sr-field__label" htmlFor="monitoring-log-source">
                Source
              </label>
              <select
                id="monitoring-log-source"
                className="sr-input"
                value={source}
                onChange={(event) => setSource(event.target.value as MonitoringLogSource)}
              >
                {LOG_SOURCES.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
              <button
                type="button"
                className="sr-btn"
                data-testid="log-reload"
                onClick={() => void loadLog(source, LOG_PAGE)}
              >
                Reload
              </button>
            </div>

            {log ? (
              <>
                <div className="sr-filterbar">
                  <div className="sr-facets" role="group" aria-label="Filter by level">
                    <button
                      type="button"
                      className={`sr-facet${logLevel === "all" ? " is-active" : ""}`}
                      aria-pressed={logLevel === "all"}
                      data-testid="log-level-all"
                      onClick={() => setLogLevel("all")}
                    >
                      All <span className="sr-facet__count">{parsedLog.length}</span>
                    </button>
                    {LOG_LEVEL_ORDER.filter((level) => counts[level] > 0).map((level) => (
                      <button
                        key={level}
                        type="button"
                        className={`sr-facet${logLevel === level ? " is-active" : ""}`}
                        aria-pressed={logLevel === level}
                        data-testid={`log-level-${level}`}
                        onClick={() => setLogLevel(level)}
                      >
                        {LOG_LEVEL_LABEL[level]}{" "}
                        <span className="sr-facet__count">{counts[level]}</span>
                      </button>
                    ))}
                  </div>
                  <label className="sr-sr-only" htmlFor="log-search">
                    Search log lines
                  </label>
                  <input
                    id="log-search"
                    className="sr-input"
                    type="search"
                    placeholder="Search lines…"
                    value={logQuery}
                    data-testid="log-search"
                    onChange={(event) => setLogQuery(event.target.value)}
                  />
                  {(logLevel !== "all" || logQuery !== "") && (
                    <button
                      type="button"
                      className="sr-btn"
                      data-testid="log-clear"
                      onClick={() => {
                        setLogLevel("all");
                        setLogQuery("");
                      }}
                    >
                      Clear filters
                    </button>
                  )}
                </div>

                <p className="sr-cell--muted" data-testid="monitoring-log-path">
                  {log.path} — showing {shownLog.length} of {log.lines.length} lines
                  {log.totalLines > log.lines.length ? ` (${log.totalLines} on disk)` : ""}
                  {logLive ? " · live" : ""}
                </p>
                <pre className="sr-log" data-testid="monitoring-log-body">
                  {shownLog.length > 0 ? shownLog.map((line) => line.text).join("\n") : "No line matches these filters."}
                </pre>
                {log.totalLines > log.lines.length && (
                  <button
                    type="button"
                    className="sr-btn"
                    data-testid="log-more"
                    onClick={() =>
                      void loadLog(source, Math.min(log.lines.length + LOG_PAGE, log.totalLines))
                    }
                  >
                    Load more lines
                  </button>
                )}
              </>
            ) : (
              <p className="sr-empty__body" data-testid="monitoring-log-empty">
                No log has been written for this source yet.
              </p>
            )}
          </section>
        </div>
      )}
    </div>
  );
}

/** A count that opens the list it counts. */
function Tile({
  label,
  value,
  hint,
  onClick,
  active,
  testId,
}: {
  label: string;
  value: React.ReactNode;
  hint?: string;
  onClick?: () => void;
  active?: boolean;
  testId?: string;
}) {
  const body = (
    <>
      <span className="sr-summary__value">{value}</span>
      <span className="sr-summary__label">{label}</span>
    </>
  );
  if (!onClick) {
    return (
      <span className="sr-summary__card" title={hint} data-testid={testId}>
        {body}
      </span>
    );
  }
  return (
    <button
      type="button"
      className={`sr-summary__card sr-summary__card--action${active ? " is-active" : ""}`}
      title={hint}
      aria-pressed={active}
      data-testid={testId}
      onClick={onClick}
    >
      {body}
    </button>
  );
}

/**
 * What one run did, release by release.
 *
 * The counts above say a run failed; this says which releases it failed on and
 * why, and offers the one action that follows from knowing: run those again.
 */
function RunLedger({
  run,
  busy,
  onRetry,
  onOpenAlbum,
}: {
  run: BackgroundRun;
  busy: boolean;
  onRetry: (runId: string) => void;
  onOpenAlbum?: (path: string) => void;
}) {
  const actionable = itemsNeedingAction(run);
  return (
    <div className="sr-section">
      <div className="sr-toolbar">
        <span className="sr-freshness" data-testid="monitoring-run-items-summary">
          {describeRunItems(run)}
        </span>
        <div className="sr-header__spacer" />
        {actionable.length > 0 && (
          <button
            type="button"
            className="sr-btn"
            disabled={busy}
            // The healthy releases of the run are deliberately not included, so
            // the label says how many releases this will actually tag.
            title="Tags the releases this run left needing action, and skips the rest"
            data-testid={`monitoring-run-retry-${run.id}`}
            onClick={() => onRetry(run.id)}
          >
            Retry {actionable.length} release{actionable.length === 1 ? "" : "s"}
          </button>
        )}
      </div>
      <table className="sr-table sr-table--log">
        <thead>
          <tr>
            <th scope="col">Release</th>
            <th scope="col">Outcome</th>
            <th scope="col">Reason</th>
            {onOpenAlbum && <th scope="col">Actions</th>}
          </tr>
        </thead>
        <tbody>
          {run.items.map((item) => (
            <tr key={item.path}>
              <td className="sr-cell--muted" title={item.path}>
                {albumNameOf(item.path)}
              </td>
              <td>
                <span className={`sr-badge ${RELEASE_CLASS[item.status]}`}>
                  {RELEASE_LABEL[item.status] ?? item.status}
                </span>
              </td>
              <td className="sr-cell--muted">{describeRunItemReason(item)}</td>
              {onOpenAlbum && (
                <td>
                  <button
                    type="button"
                    className="sr-btn"
                    onClick={() => onOpenAlbum(item.path)}
                  >
                    Open
                  </button>
                </td>
              )}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Which trigger started the task a run is currently executing. */
function triggerOf(task: WebTask, runs: MonitoringStatus["recentRuns"]): string {
  const match = runs.find((run) => run.taskId === task.taskId);
  return match ? TRIGGER_LABEL[match.trigger as BackgroundRunTrigger] ?? match.trigger : "Background";
}

/** A live run's progress as a proportion, not just two numbers to divide. */
function Progress({ task }: { task: WebTask }) {
  if (task.total <= 0) return <span className="sr-cell--muted">—</span>;
  const percent = Math.min(100, Math.round((task.progress / task.total) * 100));
  return (
    <span className="sr-progress">
      <span
        className="sr-progress__track"
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={task.total}
        aria-valuenow={task.progress}
        aria-label="Tagging progress"
      >
        <span className="sr-progress__fill" style={{ width: `${percent}%` }} />
      </span>
      <span className="sr-progress__value">
        {task.progress} / {task.total}
      </span>
    </span>
  );
}
