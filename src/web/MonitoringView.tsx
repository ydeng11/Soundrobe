/**
 * Monitoring for the browser runtime.
 *
 * The reference product's monitoring surfaces are a service-log viewer with
 * selectable sources (`gunicorn`/`worker`/`beat`/`redis`) plus background task
 * state. Soundrobe has no Celery or Redis, so this view maps those sources onto
 * the things this app genuinely writes: its own application log, its verbose
 * auto-tag trace, the scheduler state, and the task registry.
 *
 * Everything rendered here is real backend state — the schedule that will
 * actually fire, the runs that actually happened, and the releases the last
 * survey actually classified. Nothing is inferred in the browser.
 */

import React from "react";
import {
  getMonitoringStatus,
  listMonitoringReleases,
  readMonitoringLog,
  runBackgroundTagging,
  type BackgroundRun,
  type BackgroundRunStatus,
  type BackgroundRunTrigger,
  type MonitoredRelease,
  type MonitoringLog,
  type MonitoringLogSource,
  type MonitoringStatus,
  type ReleaseHealthStatus,
} from "../shared/web-adapter";

export interface MonitoringViewProps {
  /** Injected for tests. */
  loadStatus?: () => Promise<MonitoringStatus>;
  loadReleases?: () => Promise<{ releases: MonitoredRelease[] }>;
  readLog?: (source: MonitoringLogSource, lines?: number) => Promise<MonitoringLog>;
  runNow?: () => Promise<{ taskId: string }>;
  /** Poll interval; 0 disables polling (used by tests). */
  refreshMs?: number;
}

const RUN_STATUS_LABEL: Record<BackgroundRunStatus, string> = {
  running: "Running",
  completed: "Completed",
  failed: "Failed",
  cancelled: "Cancelled",
  skipped: "Skipped",
};

const RUN_STATUS_CLASS: Record<BackgroundRunStatus, string> = {
  running: "sr-badge--info",
  completed: "sr-badge--success",
  failed: "sr-badge--danger",
  cancelled: "sr-badge--muted",
  skipped: "sr-badge--muted",
};

const TRIGGER_LABEL: Record<BackgroundRunTrigger, string> = {
  schedule: "Schedule",
  manual: "Manual",
  watch: "Watched folder",
};

const RELEASE_LABEL: Record<ReleaseHealthStatus, string> = {
  healthy: "Tagged",
  needs_tagging: "Needs tagging",
  unreadable: "Unreadable",
  tagging: "Tagging",
  needs_review: "Needs review",
  failed: "Failed",
};

const RELEASE_CLASS: Record<ReleaseHealthStatus, string> = {
  healthy: "sr-badge--success",
  needs_tagging: "sr-badge--warning",
  unreadable: "sr-badge--danger",
  tagging: "sr-badge--info",
  needs_review: "sr-badge--warning",
  failed: "sr-badge--danger",
};

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

export function MonitoringView({
  loadStatus = getMonitoringStatus,
  loadReleases = listMonitoringReleases,
  readLog = readMonitoringLog,
  runNow = runBackgroundTagging,
  refreshMs = 5000,
}: MonitoringViewProps) {
  const [status, setStatus] = React.useState<MonitoringStatus | null>(null);
  const [releases, setReleases] = React.useState<MonitoredRelease[]>([]);
  const [log, setLog] = React.useState<MonitoringLog | null>(null);
  const [source, setSource] = React.useState<MonitoringLogSource>("app");
  const [error, setError] = React.useState<string | null>(null);
  const [notice, setNotice] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState(false);

  const refresh = React.useCallback(async () => {
    try {
      const [next, monitored] = await Promise.all([loadStatus(), loadReleases()]);
      setStatus(next);
      setReleases(monitored.releases);
      setError(null);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  }, [loadStatus, loadReleases]);

  React.useEffect(() => {
    void refresh();
  }, [refresh]);

  React.useEffect(() => {
    if (refreshMs <= 0) return undefined;
    const timer = window.setInterval(() => void refresh(), refreshMs);
    return () => window.clearInterval(timer);
  }, [refresh, refreshMs]);

  React.useEffect(() => {
    let active = true;
    void (async () => {
      try {
        const next = await readLog(source);
        if (active) setLog(next);
      } catch (reason) {
        if (!active) return;
        // A source that has never written a file is an ordinary state, not a
        // failure of the page.
        setLog(null);
        setNotice(reason instanceof Error ? reason.message : String(reason));
      }
    })();
    return () => {
      active = false;
    };
  }, [readLog, source]);

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

  const runs = status?.recentRuns ?? [];
  const running = status?.running ?? [];
  const needsTagging = releases.filter((release) => release.status === "needs_tagging").length;

  return (
    <div className="sr-pane" data-testid="monitoring">
      <div className="sr-toolbar">
        <h2 className="sr-section__title">Monitoring</h2>
        <div className="sr-header__spacer" />
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

      <div className="sr-summary" data-testid="monitoring-summary">
        <Summary label="Background tagging" value={status?.enabled ? "On" : "Off"} />
        <Summary label="Next run" value={formatStamp(status?.nextRunAt ?? null)} />
        <Summary label="Running" value={running.length} />
        <Summary label="Needs tagging" value={needsTagging} />
      </div>

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

      <section className="sr-section" data-testid="monitoring-schedule">
        <h3 className="sr-section__title">Schedule</h3>
        {status ? (
          <div className="sr-technical">
            <Row label="Enabled" value={status.enabled ? "Yes" : "No"} />
            <Row label="Crontab" value={status.schedule} mono />
            <Row label="Next run" value={formatStamp(status.nextRunAt)} />
            <Row label="Watch new files" value={status.watchEnabled ? "Yes" : "No"} />
            <Row
              label="Skip fully tagged"
              value={status.skipUnchanged ? "Yes" : "No"}
            />
            <Row
              label="Coverage"
              value={
                status.effectiveDirectories.length > 0
                  ? status.effectiveDirectories.join(", ")
                  : "No directory is available"
              }
            />
          </div>
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
                  <th scope="col">Task</th>
                  <th scope="col">Progress</th>
                  <th scope="col">Step</th>
                </tr>
              </thead>
              <tbody>
                {running.map((task) => (
                  <tr key={task.taskId}>
                    <td>{task.taskId}</td>
                    <td>{task.total > 0 ? `${task.progress} / ${task.total}` : "—"}</td>
                    <td className="sr-cell--muted">{task.message || "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

      <section className="sr-section" data-testid="monitoring-runs">
        <h3 className="sr-section__title">Recent runs</h3>
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
                  <th scope="col">Tagged</th>
                  <th scope="col">Failed</th>
                  <th scope="col">Summary</th>
                </tr>
              </thead>
              <tbody>
                {runs.map((run) => (
                  <tr key={run.id}>
                    <td>{formatStamp(run.startedAt)}</td>
                    <td>{TRIGGER_LABEL[run.trigger] ?? run.trigger}</td>
                    <td>
                      <span className={`sr-badge ${RUN_STATUS_CLASS[run.status]}`}>
                        {RUN_STATUS_LABEL[run.status] ?? run.status}
                      </span>
                    </td>
                    <td>{run.tagged}</td>
                    <td>{run.failed}</td>
                    <td className="sr-cell--muted">{run.message || "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

      <section className="sr-section" data-testid="monitoring-releases">
        <h3 className="sr-section__title">Releases</h3>
        {releases.length === 0 ? (
          <p className="sr-empty__body">No release has been surveyed yet.</p>
        ) : (
          <div className="sr-table-wrap">
            <table className="sr-table sr-table--log">
              <thead>
                <tr>
                  <th scope="col">Release</th>
                  <th scope="col">Tagging state</th>
                </tr>
              </thead>
              <tbody>
                {releases.map((release) => (
                  <tr key={release.path}>
                    <td className="sr-cell--muted">{release.path}</td>
                    <td>
                      <span className={`sr-badge ${RELEASE_CLASS[release.status]}`}>
                        {RELEASE_LABEL[release.status] ?? release.status}
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

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
        </div>
        {log ? (
          <>
            <p className="sr-cell--muted" data-testid="monitoring-log-path">
              {log.path} — showing {log.lines.length} of {log.totalLines} lines
            </p>
            <pre className="sr-log" data-testid="monitoring-log-body">
              {log.lines.length > 0 ? log.lines.join("\n") : "Log is empty."}
            </pre>
          </>
        ) : (
          <p className="sr-empty__body" data-testid="monitoring-log-empty">
            No log has been written for this source yet.
          </p>
        )}
      </section>
    </div>
  );
}

function Summary({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <span className="sr-summary__card">
      <span className="sr-summary__value">{value}</span>
      <span className="sr-summary__label">{label}</span>
    </span>
  );
}

function Row({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="sr-technical__row">
      <span className="sr-technical__label">{label}</span>
      <span className={mono ? "sr-log-inline" : undefined}>{value}</span>
    </div>
  );
}
