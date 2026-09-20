/**
 * Pure presentation logic for the Monitoring view.
 *
 * Kept out of the component so each rule can be tested on its own: time
 * arithmetic, facet counts, list filtering, cron wording and log level
 * detection are all decisions with a wrong answer, not formatting.
 *
 * Two rules this module will not break:
 *
 * - `describeCron` returns `null` when it cannot be *sure* of a wording. A
 *   confidently wrong description of a schedule is worse than no description,
 *   so an unusual expression falls back to being shown verbatim.
 * - Nothing here decides when a run fires. The service owns cron semantics; this
 *   only puts the stored expression into words.
 */

import type {
  BackgroundRun,
  BackgroundRunItem,
  BackgroundRunStatus,
  BackgroundRunTrigger,
  MonitoredRelease,
  ReleaseHealthStatus,
} from "../shared/web-adapter";

export const RELEASE_LABEL: Record<ReleaseHealthStatus, string> = {
  healthy: "Tagged",
  needs_tagging: "Needs tagging",
  unreadable: "Unreadable",
  tagging: "Tagging",
  needs_review: "Needs review",
  failed: "Failed",
};

export const RELEASE_CLASS: Record<ReleaseHealthStatus, string> = {
  healthy: "sr-badge--success",
  needs_tagging: "sr-badge--warning",
  unreadable: "sr-badge--danger",
  tagging: "sr-badge--info",
  needs_review: "sr-badge--warning",
  failed: "sr-badge--danger",
};

export const RUN_STATUS_LABEL: Record<BackgroundRunStatus, string> = {
  running: "Running",
  completed: "Completed",
  failed: "Failed",
  cancelled: "Cancelled",
  skipped: "Skipped",
};

export const RUN_STATUS_CLASS: Record<BackgroundRunStatus, string> = {
  running: "sr-badge--info",
  completed: "sr-badge--success",
  failed: "sr-badge--danger",
  cancelled: "sr-badge--muted",
  skipped: "sr-badge--muted",
};

export const TRIGGER_LABEL: Record<BackgroundRunTrigger, string> = {
  schedule: "Schedule",
  manual: "Manual",
  watch: "Watched folder",
};

/**
 * The releases of a run an operator can still do something about.
 *
 * This is the complement of "it worked", and it is deliberately what the retry
 * action sends: re-tagging a release whose tags were already written would make
 * "retry the failures" mean "tag the library again".
 */
export function itemsNeedingAction(run: BackgroundRun): BackgroundRunItem[] {
  return run.items.filter(
    (item) => item.status !== "healthy" && item.status !== "tagging",
  );
}

/**
 * How a run's ledger reads in one line.
 *
 * `itemsTruncated` means the run covered more releases than the ledger stores,
 * which the reader has to know before treating the list as complete. The wording
 * claims an *order*, not a guarantee: when a run leaves more releases needing
 * action than the ledger holds, even some of those are dropped, and saying
 * "all failures kept" would be a promise the page cannot keep.
 */
export function describeRunItems(run: BackgroundRun): string {
  const kept = `${run.items.length} release${run.items.length === 1 ? "" : "s"} recorded`;
  return run.itemsTruncated
    ? `${kept} — failures and reviews first, later successes dropped`
    : kept;
}

/**
 * Why a release ended up the way it did: the reason code, the source that
 * supplied the tags, or both. Absent for a release that simply worked.
 */
export function describeRunItemReason(item: BackgroundRunItem): string {
  const parts = [item.reason, item.authority ? `via ${item.authority}` : null].filter(
    (part): part is string => Boolean(part),
  );
  return parts.length > 0 ? parts.join(" · ") : "—";
}

/**
 * The order facets are shown in: what needs attention first, then what is fine.
 * A facet list ordered by enum declaration would bury "Needs review".
 */
const RELEASE_FACET_ORDER: ReleaseHealthStatus[] = [
  "needs_review",
  "failed",
  "unreadable",
  "needs_tagging",
  "tagging",
  "healthy",
];

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** Largest unit that fits, so "2d" never reads as "48h". */
function coarse(ms: number): string {
  const abs = Math.abs(ms);
  const pair = (major: number, majorUnit: string, minor: number, minorUnit: string): string =>
    minor === 0 ? `${major}${majorUnit}` : `${major}${majorUnit} ${minor}${minorUnit}`;
  if (abs >= DAY) return pair(Math.floor(abs / DAY), "d", Math.floor((abs % DAY) / HOUR), "h");
  if (abs >= HOUR) return pair(Math.floor(abs / HOUR), "h", Math.floor((abs % HOUR) / MINUTE), "m");
  if (abs >= MINUTE) return pair(Math.floor(abs / MINUTE), "m", Math.floor((abs % MINUTE) / 1000), "s");
  return `${Math.max(0, Math.round(abs / 1000))}s`;
}

/**
 * "in 2h 14m" / "12m ago" / "just now".
 *
 * A countdown is what makes a schedule legible; an absolute timestamp makes the
 * reader do the subtraction. Returns the raw value when it cannot be parsed,
 * rather than inventing a time.
 */
export function relativeTime(value: string | null, now: Date = new Date()): string {
  if (!value) return "—";
  const at = new Date(value);
  if (Number.isNaN(at.getTime())) return value;
  const delta = at.getTime() - now.getTime();
  if (Math.abs(delta) < 45_000) return "just now";
  return delta > 0 ? `in ${coarse(delta)}` : `${coarse(delta)} ago`;
}

/**
 * How long a run took, or how long it has been going.
 *
 * Named for the run rather than the duration, because `TrackDetailEditor` also
 * exports a `formatDuration`, which formats an audio length in seconds as
 * `m:ss`. Two functions of one name with different meanings is a mistake waiting
 * to be made.
 *
 * `finishedAt` has been recorded since background tagging shipped and was never
 * shown; this is the metric that tells you whether a run is stuck.
 */
export function formatRunDuration(
  startedAt: string,
  finishedAt: string | null,
  now: Date = new Date(),
): string {
  const start = new Date(startedAt);
  if (Number.isNaN(start.getTime())) return "—";
  const end = finishedAt ? new Date(finishedAt) : now;
  if (Number.isNaN(end.getTime())) return "—";
  const ms = end.getTime() - start.getTime();
  if (ms < 0) return "—";
  if (ms < 1000) return "<1s";
  return coarse(ms);
}

export interface ReleaseFacet {
  status: ReleaseHealthStatus;
  label: string;
  count: number;
}

/**
 * Counts per status, in attention order, always including the answered statuses
 * so the list looks the same whether or not each one has members.
 */
export function facetCounts(releases: MonitoredRelease[]): ReleaseFacet[] {
  const counts = new Map<ReleaseHealthStatus, number>();
  for (const release of releases) {
    counts.set(release.status, (counts.get(release.status) ?? 0) + 1);
  }
  return RELEASE_FACET_ORDER.filter((status) => counts.has(status)).map((status) => ({
    status,
    label: RELEASE_LABEL[status],
    count: counts.get(status) ?? 0,
  }));
}

/**
 * Filter by status and by a case-insensitive substring of the album path.
 *
 * The path is the only identity a release has, so it is what a search has to
 * work on.
 */
export function filterReleases(
  releases: MonitoredRelease[],
  status: ReleaseHealthStatus | "all",
  query: string,
): MonitoredRelease[] {
  const needle = query.trim().toLowerCase();
  return releases.filter((release) => {
    if (status !== "all" && release.status !== status) return false;
    if (needle === "") return true;
    return release.path.toLowerCase().includes(needle);
  });
}

export type LogLevel = "error" | "warn" | "info" | "debug" | "unknown";

export const LOG_LEVEL_ORDER: LogLevel[] = ["error", "warn", "info", "debug", "unknown"];

export const LOG_LEVEL_LABEL: Record<LogLevel, string> = {
  error: "Error",
  warn: "Warning",
  info: "Info",
  debug: "Debug",
  unknown: "Other",
};

/**
 * Which of our own log levels a line carries.
 *
 * `tracing` writes `LEVEL` as a bare token and often as a `level=INFO`
 * field, so both shapes are matched. Anything unrecognised is `unknown` rather
 * than being assumed to be info: mislabelling an error as info is how a real
 * failure gets filtered out of view.
 */
export function levelOf(line: string): LogLevel {
  const qualified = line.match(/\blevel\s*[=:]\s*"?([A-Za-z]+)"?/);
  const token = qualified?.[1] ?? line.match(/\b(ERROR|FATAL|WARN|INFO|DEBUG|TRACE)\b/)?.[1];
  switch ((token ?? "").toUpperCase()) {
    case "ERROR":
    case "FATAL":
      return "error";
    case "WARN":
    case "WARNING":
      return "warn";
    case "INFO":
      return "info";
    case "DEBUG":
    case "TRACE":
      return "debug";
    default:
      return "unknown";
  }
}

export interface LogLine {
  level: LogLevel;
  text: string;
}

export function parseLog(lines: string[]): LogLine[] {
  return lines.map((text) => ({ level: levelOf(text), text }));
}

/** Filter parsed lines by level and by a case-insensitive substring. */
export function filterLog(
  lines: LogLine[],
  level: LogLevel | "all",
  query: string,
): LogLine[] {
  const needle = query.trim().toLowerCase();
  return lines.filter((line) => {
    if (level !== "all" && line.level !== level) return false;
    if (needle === "") return true;
    return line.text.toLowerCase().includes(needle);
  });
}

/** Counts per level, so a filter can say how much it would show before you click. */
export function levelCounts(lines: LogLine[]): Record<LogLevel, number> {
  const counts: Record<LogLevel, number> = {
    error: 0,
    warn: 0,
    info: 0,
    debug: 0,
    unknown: 0,
  };
  for (const line of lines) counts[line.level] += 1;
  return counts;
}

/**
 * The sentence a person would say instead of the expression.
 *
 * Returns `null` for anything it does not recognise exactly — a confidently
 * wrong schedule description is worse than showing the raw fields, because the
 * reader stops checking.
 */
export function describeCron(expression: string): string | null {
  const fields = expression.trim().split(/\s+/);
  if (fields.length !== 5) return null;
  const [minute, hour, dom, month, dow] = fields as [string, string, string, string, string];

  const fixed = (field: string): number | null =>
    /^\d{1,2}$/.test(field) ? Number(field) : null;
  const every = (field: string): number | null =>
    /^\*$/.test(field) ? 1 : /^\*\/(\d{1,2})$/.test(field) ? Number(field.slice(2)) : null;

  const minuteEvery = every(minute);
  const hourEvery = every(hour);
  const dayNames = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

  // A pure minute interval. `* */2 * * *` deliberately falls through to null:
  // it fires every minute of every second hour, and calling that "every 2 hours"
  // would understate it by a factor of sixty.
  if (minuteEvery !== null && hour === "*" && dom === "*" && month === "*" && dow === "*") {
    return minuteEvery === 1 ? "Every minute" : `Every ${minuteEvery} minutes`;
  }

  const minuteAt = fixed(minute);
  const hourAt = fixed(hour);

  // A fixed minute inside an hourly cadence.
  if (minuteAt !== null && hourEvery !== null && dom === "*" && month === "*" && dow === "*") {
    return hourEvery === 1
      ? `Hourly at minute ${minuteAt}`
      : `Every ${hourEvery} hours at minute ${minuteAt}`;
  }

  // A single time each day, plus the two common narrower cases.
  if (minuteAt !== null && hourAt !== null && month === "*") {
    const time = `${String(hourAt).padStart(2, "0")}:${String(minuteAt).padStart(2, "0")}`;
    if (dom === "*" && dow === "*") return `Every day at ${time}`;
    const weekday = fixed(dow);
    if (dom === "*" && weekday !== null && dayNames[weekday]) {
      return `Every ${dayNames[weekday]} at ${time}`;
    }
    const day = fixed(dom);
    if (day !== null && dow === "*") return `Day ${day} of the month at ${time}`;
  }

  return null;
}
