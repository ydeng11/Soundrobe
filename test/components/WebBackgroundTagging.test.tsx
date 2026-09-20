// @vitest-environment jsdom
/**
 * Background tagging, Monitoring and Settings.
 *
 * These tests encode *why* the surface exists rather than just what it renders:
 * an unattended writer must be configurable, its schedule must not be storable
 * in a state where it can never fire, its history must reflect runs that
 * really happened, and a refusal must be visible instead of silent.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import React from "react";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MonitoringView } from "../../src/web/MonitoringView";
import { SettingsView, SCHEDULE_PRESETS } from "../../src/web/SettingsView";
import { WorkspaceSettingsDrawer } from "../../src/web/WorkspaceSettingsDrawer";
import { TrackTable } from "../../src/web/TrackTable";
import type {
  BackgroundTaggingSettings,
  MonitoringLog,
  MonitoringStatus,
} from "../../src/shared/web-adapter";
import type { TrackData } from "../../src/shared/desktop-api";

afterEach(() => cleanup());

const SETTINGS: BackgroundTaggingSettings = {
  enabled: true,
  schedule: "0 0 * * *",
  directories: [],
  watchEnabled: false,
  skipUnchanged: true,
};

function status(overrides: Partial<MonitoringStatus> = {}): MonitoringStatus {
  return {
    ...SETTINGS,
    nextRunAt: "2026-09-19 0:00:00.0",
    effectiveDirectories: ["/library/Music"],
    running: [],
    recentRuns: [],
    ...overrides,
  };
}

function emptyLog(): MonitoringLog {
  return { path: "/data/auto-tagger.log", totalLines: 0, lines: [] };
}

describe("Monitoring", () => {
  it("keeps the operation log reachable here, since it has no navigation entry", async () => {
    // The operations answer the same question this view does - what has the
    // background tagger been doing - so they live here. If this tab disappeared,
    // operations would be unreachable from anywhere in the app.
    render(
      <MonitoringView
        refreshMs={0}
        loadStatus={async () => status()}
        loadReleases={async () => ({ releases: [] })}
        readLog={async () => emptyLog()}
        loadTasks={async () => [
          { taskId: "task-1", status: "running", progress: 3, total: 10, message: "Working", result: null },
          { taskId: "task-2", status: "failed", progress: 0, total: 10, message: "Boom", result: null },
        ]}
        cancelTask={async () => ({ ok: true })}
      />,
    );

    // Operations are not on the first tab: the page is four questions, and this
    // is one of them rather than the default.
    expect(screen.queryByTestId("operation-log")).toBeNull();
    fireEvent.click(await screen.findByTestId("monitoring-tab-operations"));

    const section = await screen.findByTestId("operation-log");
    expect(within(section).getByRole("heading", { name: "Operations" })).toBeTruthy();
    const table = within(section).getByTestId("log-table");
    expect(within(table).getByText("Running")).toBeTruthy();
    expect(within(table).getByText("Failed")).toBeTruthy();
  });

  it("shows kept and cleared tags when a tagging operation finishes", async () => {
    render(
      <MonitoringView
        refreshMs={0}
        loadStatus={async () => status()}
        loadReleases={async () => ({ releases: [] })}
        readLog={async () => emptyLog()}
        loadTasks={async () => [
          {
            taskId: "task-complete",
            status: "completed",
            progress: 9,
            total: 9,
            message: "Tagged 3 files",
            result: { keptTags: ["Title", "Album"], clearedTags: ["Comment", "Description"] },
          },
        ]}
      />,
    );

    fireEvent.click(await screen.findByTestId("monitoring-tab-operations"));
    const summary = await screen.findByTestId("operation-tag-summary");
    expect(summary.textContent).toContain("Tags kept: Title, Album");
    expect(summary.textContent).toContain("Tags cleared: Comment, Description");
  });

  it("answers one question per tab instead of stacking every section", async () => {
    // Six stacked sections made the reader scroll past four irrelevant ones to
    // reach the one they wanted; each tab now owns exactly one question.
    render(
      <MonitoringView
        refreshMs={0}
        loadStatus={async () => status()}
        loadReleases={async () => ({ releases: [] })}
        readLog={async () => emptyLog()}
      />,
    );

    for (const id of ["overview", "releases", "operations", "logs"]) {
      expect(await screen.findByTestId(`monitoring-tab-${id}`)).toBeTruthy();
    }
    // Overview is the landing tab, so the schedule is readable without a click.
    expect(await screen.findByTestId("monitoring-schedule")).toBeTruthy();
    expect(screen.queryByTestId("monitoring-releases")).toBeNull();
    expect(screen.queryByTestId("monitoring-logs")).toBeNull();

    fireEvent.click(screen.getByTestId("monitoring-tab-releases"));
    expect(await screen.findByTestId("monitoring-releases")).toBeTruthy();
    // And the other panels do not linger behind it.
    expect(screen.queryByTestId("monitoring-schedule")).toBeNull();
  });

  it("reports the schedule that will actually fire and the coverage", async () => {
    render(
      <MonitoringView
        refreshMs={0}
        loadStatus={async () => status()}
        loadReleases={async () => ({ releases: [] })}
        readLog={async () => emptyLog()}
      />,
    );

    expect(await screen.findByTestId("monitoring-schedule")).toBeTruthy();
    expect(screen.getByText("0 0 * * *")).toBeTruthy();
    expect(screen.getByText("/library/Music")).toBeTruthy();
  });

  it("surfaces a refused run instead of silently doing nothing", async () => {
    // The service refuses when tagging is disabled, busy, or has nothing to
    // cover. Swallowing that would look identical to a successful start.
    render(
      <MonitoringView
        refreshMs={0}
        loadStatus={async () => status({ enabled: false })}
        loadReleases={async () => ({ releases: [] })}
        readLog={async () => emptyLog()}
        runNow={async () => {
          throw new Error("background tagging is disabled");
        }}
      />,
    );

    const button = await screen.findByRole("button", { name: "Tag now" });
    expect((button as HTMLButtonElement).disabled).toBe(true);
  });

  it("reports a start that was refused while still enabled", async () => {
    render(
      <MonitoringView
        refreshMs={0}
        loadStatus={async () => status()}
        loadReleases={async () => ({ releases: [] })}
        readLog={async () => emptyLog()}
        runNow={async () => {
          throw new Error("another auto-tag operation is already running");
        }}
      />,
    );

    fireEvent.click(await screen.findByRole("button", { name: "Tag now" }));
    expect(
      await screen.findByText("another auto-tag operation is already running"),
    ).toBeTruthy();
  });

  it("renders recorded runs with their outcome", async () => {
    render(
      <MonitoringView
        refreshMs={0}
        loadStatus={async () =>
          status({
            recentRuns: [
              {
                id: "bg-run-1",
                trigger: "schedule",
                startedAt: "2026-09-19 0:00:00.0",
                finishedAt: "2026-09-19 0:04:00.0",
                status: "completed",
                tagged: 7,
                failed: 0,
                message: "Tagged 7 of 9",
                taskId: "background-auto-tag-1",
                items: [],
                itemsTruncated: false,
              },
            ],
          })
        }
        loadReleases={async () => ({ releases: [] })}
        readLog={async () => emptyLog()}
      />,
    );

    expect(await screen.findByTestId("monitoring-runs-table")).toBeTruthy();
    expect(screen.getByText("Tagged 7 of 9")).toBeTruthy();
    // The trigger is labelled per row, so a scheduled run is distinguishable
    // from a manual or watched one.
    expect(within(screen.getByTestId("monitoring-runs-table")).getByText("Schedule")).toBeTruthy();
  });

  it("shows which release a run failed on and why, which the counters cannot say", async () => {
    // "Tagged 1, 1 failed" tells an operator that something went wrong and
    // nothing about what. Without the per-release ledger they would be reading
    // the log file to find out.
    const retryFailed = vi.fn(async () => ({ taskId: "background-auto-tag-9" }));
    render(
      <MonitoringView
        refreshMs={0}
        loadStatus={async () =>
          status({
            recentRuns: [
              {
                id: "bg-run-1",
                trigger: "schedule",
                startedAt: "2026-09-19T00:00:00.000Z",
                finishedAt: "2026-09-19T00:04:00.000Z",
                status: "failed",
                tagged: 1,
                failed: 1,
                message: "Tagged 1, 1 failed: permission denied",
                taskId: null,
                items: [
                  { path: "/library/Album One", status: "healthy", authority: "discogs" },
                  {
                    path: "/library/Album Two",
                    status: "failed",
                    reason: "permission denied",
                  },
                ],
                itemsTruncated: false,
              },
            ],
          })
        }
        loadReleases={async () => ({ releases: [] })}
        readLog={async () => emptyLog()}
        retryFailed={retryFailed}
      />,
    );

    // The ledger is not in the way until it is asked for: the table stays a
    // one-line-per-run summary.
    await screen.findByTestId("monitoring-runs-table");
    expect(screen.queryByTestId("monitoring-run-detail-bg-run-1")).toBeNull();
    fireEvent.click(screen.getByTestId("monitoring-run-items-bg-run-1"));

    const detail = await screen.findByTestId("monitoring-run-detail-bg-run-1");
    expect(within(detail).getByText("Album Two")).toBeTruthy();
    expect(within(detail).getByText("permission denied")).toBeTruthy();
    // The source that supplied the tags is part of the answer to "what happened".
    expect(within(detail).getByText("via discogs")).toBeTruthy();
    // Only the release that needs action is retryable, so the button cannot send
    // the healthy one back through the tagger.
    expect(within(detail).getByText("Retry 1 release")).toBeTruthy();
  });

  it("retries the failed releases of the run it was opened from", async () => {
    const retryFailed = vi.fn(async () => ({ taskId: "background-auto-tag-9" }));
    render(
      <MonitoringView
        refreshMs={0}
        loadStatus={async () =>
          status({
            recentRuns: [
              {
                id: "bg-run-1",
                trigger: "schedule",
                startedAt: "2026-09-19T00:00:00.000Z",
                finishedAt: "2026-09-19T00:04:00.000Z",
                status: "failed",
                tagged: 0,
                failed: 2,
                message: "Tagged 0, 2 failed",
                taskId: null,
                items: [
                  { path: "/library/Album One", status: "failed", reason: "permission denied" },
                  { path: "/library/Album Two", status: "unreadable" },
                ],
                itemsTruncated: false,
              },
            ],
          })
        }
        loadReleases={async () => ({ releases: [] })}
        readLog={async () => emptyLog()}
        retryFailed={retryFailed}
      />,
    );

    fireEvent.click(await screen.findByTestId("monitoring-run-items-bg-run-1"));
    fireEvent.click(await screen.findByTestId("monitoring-run-retry-bg-run-1"));

    // The run id is what scopes the retry; without it the service would fall
    // back to the newest run and could retry something else entirely.
    await waitFor(() => expect(retryFailed).toHaveBeenCalledWith("bg-run-1"));
    // A refusal from the service has to reach the reader, not vanish.
    expect(await screen.findByText(/Retrying the releases that failed/)).toBeTruthy();
  });

  it("says when the ledger dropped releases rather than reading as complete", async () => {
    render(
      <MonitoringView
        refreshMs={0}
        loadStatus={async () =>
          status({
            recentRuns: [
              {
                id: "bg-run-1",
                trigger: "manual",
                startedAt: "2026-09-19T00:00:00.000Z",
                finishedAt: "2026-09-19T00:04:00.000Z",
                status: "completed",
                tagged: 400,
                failed: 0,
                message: "Tagged 400 of 400",
                taskId: null,
                items: [{ path: "/library/Album One", status: "healthy" }],
                itemsTruncated: true,
              },
            ],
          })
        }
        loadReleases={async () => ({ releases: [] })}
        readLog={async () => emptyLog()}
      />,
    );

    fireEvent.click(await screen.findByTestId("monitoring-run-items-bg-run-1"));
    const summary = await screen.findByTestId("monitoring-run-items-summary");
    expect(summary.textContent).toContain("dropped");
  });

  it("says so when no run has happened rather than showing an empty table", async () => {
    render(
      <MonitoringView
        refreshMs={0}
        loadStatus={async () => status()}
        loadReleases={async () => ({ releases: [] })}
        readLog={async () => emptyLog()}
      />,
    );
    expect(await screen.findByTestId("monitoring-runs-empty")).toBeTruthy();
  });

  it("classifies surveyed releases so untagged music is visible", async () => {
    render(
      <MonitoringView
        refreshMs={0}
        loadStatus={async () => status()}
        loadReleases={async () => ({
          releases: [
            { path: "/library/Album One", status: "needs_tagging" },
            { path: "/library/Album Two", status: "healthy" },
          ],
        })}
        readLog={async () => emptyLog()}
      />,
    );

    fireEvent.click(await screen.findByTestId("monitoring-tab-releases"));
    const panel = await screen.findByTestId("monitoring-releases");
    // Every surveyed release is listed, so untagged music cannot hide.
    expect(within(panel).getByText("/library/Album One")).toBeTruthy();
    expect(within(panel).getByText("/library/Album Two")).toBeTruthy();
    // The label appears twice by design - once on the count tile, once on the
    // facet - and both must agree on how many albums are in that state.
    expect(screen.getByTestId("release-filter-needs_tagging").textContent).toContain("1");
  });

  it("turns a count into the way to that list, rather than a number to note", async () => {
    // This is the whole point of the overview strip: a reader who sees "1 needs
    // tagging" should be one click from the album, not left to search for it.
    render(
      <MonitoringView
        refreshMs={0}
        loadStatus={async () => status()}
        loadReleases={async () => ({
          releases: [
            { path: "/library/Album One", status: "needs_tagging" },
            { path: "/library/Album Two", status: "healthy" },
          ],
        })}
        readLog={async () => emptyLog()}
      />,
    );

    fireEvent.click(await screen.findByTestId("monitoring-tile-needs_tagging"));

    // The click lands on the filtered list, not merely on the releases tab.
    const panel = await screen.findByTestId("monitoring-releases");
    expect(within(panel).getByText("/library/Album One")).toBeTruthy();
    expect(within(panel).queryByText("/library/Album Two")).toBeNull();
    expect(screen.getByTestId("release-count").textContent).toContain("Showing 1 of 1");

    // And clearing the filter gives the whole library back.
    fireEvent.click(screen.getByTestId("release-clear"));
    expect(within(await screen.findByTestId("monitoring-releases")).getByText("/library/Album Two")).toBeTruthy();
  });

  it("gives every release row a way into the album it names", async () => {
    // Monitoring can say an album needs attention; without this it can do
    // nothing about it.
    const onOpenAlbum = vi.fn();
    render(
      <MonitoringView
        refreshMs={0}
        loadStatus={async () => status()}
        loadReleases={async () => ({
          releases: [{ path: "/library/Album One", status: "needs_review" }],
        })}
        readLog={async () => emptyLog()}
        onOpenAlbum={onOpenAlbum}
      />,
    );

    fireEvent.click(await screen.findByTestId("monitoring-tile-needs_review"));
    fireEvent.click(await screen.findByTestId("release-open-/library/Album One"));
    expect(onOpenAlbum).toHaveBeenCalledWith("/library/Album One");
  });

  it("bounds a large library instead of rendering all of it", async () => {
    // An unbounded list is both a UX problem and a rendering one; a real library
    // surveys into the thousands.
    const many = Array.from({ length: 120 }, (_, index) => ({
      path: `/library/Album ${String(index).padStart(3, "0")}`,
      status: "healthy" as const,
    }));
    render(
      <MonitoringView
        refreshMs={0}
        loadStatus={async () => status()}
        loadReleases={async () => ({ releases: many })}
        readLog={async () => emptyLog()}
      />,
    );

    fireEvent.click(await screen.findByTestId("monitoring-tab-releases"));
    expect(screen.getByTestId("release-count").textContent).toContain("Showing 50 of 120");
    expect(document.querySelectorAll('[data-testid="monitoring-releases"] tbody tr')).toHaveLength(50);

    fireEvent.click(screen.getByTestId("release-show-more"));
    expect(screen.getByTestId("release-count").textContent).toContain("Showing 100 of 120");
  });

  it("shows the log for the selected source", async () => {
    const readLog = vi.fn(async (source: string) =>
      source === "app"
        ? { path: "/data/auto-tagger.log", totalLines: 2, lines: ["hello", "world"] }
        : { path: "/data/auto-tag-debug.log", totalLines: 1, lines: ["verbose"] },
    );
    render(
      <MonitoringView
        refreshMs={0}
        loadStatus={async () => status()}
        loadReleases={async () => ({ releases: [] })}
        readLog={readLog}
      />,
    );

    fireEvent.click(await screen.findByTestId("monitoring-tab-logs"));
    expect(await screen.findByText(/hello/)).toBeTruthy();
    fireEvent.change(screen.getByLabelText("Source"), { target: { value: "auto-tag" } });
    expect(await screen.findByText("verbose")).toBeTruthy();
    expect(readLog).toHaveBeenCalledWith("auto-tag", 500);
  });

  it("filters the log by level, so an error is not buried in routine output", async () => {
    render(
      <MonitoringView
        refreshMs={0}
        loadStatus={async () => status()}
        loadReleases={async () => ({ releases: [] })}
        readLog={async () => ({
          path: "/data/auto-tagger.log",
          totalLines: 3,
          lines: ["INFO started", "ERROR could not write", "INFO done"],
        })}
      />,
    );

    fireEvent.click(await screen.findByTestId("monitoring-tab-logs"));
    expect(await screen.findByText(/ERROR could not write/)).toBeTruthy();

    // The count on the facet says how much it would show before you commit to it.
    expect(screen.getByTestId("log-level-error").textContent).toContain("1");
    fireEvent.click(screen.getByTestId("log-level-error"));

    const body = await screen.findByTestId("monitoring-log-body");
    expect(body.textContent).toContain("ERROR could not write");
    expect(body.textContent).not.toContain("INFO started");
  });

  it("states the clock the schedule is measured against instead of leaving it to be guessed", async () => {
    render(
      <MonitoringView
        refreshMs={0}
        loadStatus={async () => ({ ...status(), schedule: "0 0 * * *", nextRunAt: "2026-09-20T00:00:00.000Z" })}
        loadReleases={async () => ({ releases: [] })}
        readLog={async () => emptyLog()}
      />,
    );

    // A five-field expression is not readable, so the page says what it means.
    expect(await screen.findByText("Every day at 00:00")).toBeTruthy();
    expect(screen.getByTestId("monitoring-timezone").textContent).toContain("browser");
  });

  it("shows how long a run took, which the record has always carried", async () => {
    render(
      <MonitoringView
        refreshMs={0}
        loadStatus={async () => ({
          ...status(),
          recentRuns: [
            {
              id: "run-1",
              trigger: "schedule",
              startedAt: "2026-09-19T10:00:00.000Z",
              finishedAt: "2026-09-19T10:01:20.000Z",
              status: "completed",
              tagged: 2,
              failed: 0,
              message: "done",
              taskId: null,
              items: [],
              itemsTruncated: false,
            },
          ],
        })}
        loadReleases={async () => ({ releases: [] })}
        readLog={async () => emptyLog()}
      />,
    );

    const table = await screen.findByTestId("monitoring-runs-table");
    // Duration is the metric that tells you whether a run crawled or got stuck.
    expect(within(table).getByText("1m 20s")).toBeTruthy();
  });

  it("treats a source with no log yet as an ordinary state", async () => {
    render(
      <MonitoringView
        refreshMs={0}
        loadStatus={async () => status()}
        loadReleases={async () => ({ releases: [] })}
        readLog={async () => {
          throw new Error("no auto-tag log yet");
        }}
      />,
    );
    fireEvent.click(await screen.findByTestId("monitoring-tab-logs"));
    expect(await screen.findByTestId("monitoring-log-empty")).toBeTruthy();
  });
});

describe("Settings", () => {
  it("gives every checkbox setting the whole row for its name", async () => {
    // A checkbox field's label *is* its control, and these labels are sentences:
    // inside the label gutter "Let the assistant apply its own actions without
    // confirmation" was ellipsised to "Let the assistant apply its own ac…".
    render(
      <SettingsView
        loadBackground={async () => SETTINGS}
        loadConfig={async () => ({})}
        loadAppInfo={async () => ({ identifier: "app", version: "1", runtime: "web", dev: false })}
      />,
    );

    await screen.findByTestId("settings");
    for (const id of [
      "setting-remote-lookup",
      "setting-discogs",
      "setting-lyrics",
      "setting-background-enabled",
      "setting-watch",
      "setting-skip-unchanged",
      "setting-assistant",
      "setting-debug",
    ]) {
      const box = document.getElementById(id) as HTMLInputElement;
      expect(box, id).toBeTruthy();
      expect(box.closest(".sr-field")?.className, id).toContain("sr-field--stacked");
    }
  });

  it("suggests supported cleanup tags and shows the tags that will remain", async () => {
    const saveConfig = vi.fn(async () => undefined);
    const suggestTags = vi.fn(async (query: string) => {
      if (query.toLowerCase().includes("desc")) {
        return [{ id: "description", label: "Description", frame: "TXXX:DESCRIPTION", source: "ID3 / Navidrome" }];
      }
      return [
        { id: "comment", label: "Comment", frame: "COMM", source: "ID3 / Navidrome" },
        { id: "description", label: "Description", frame: "TXXX:DESCRIPTION", source: "ID3 / Navidrome" },
        { id: "title", label: "Title", frame: "TIT2", source: "ID3 / Navidrome" },
      ];
    });
    render(
      <SettingsView
        loadBackground={async () => SETTINGS}
        loadConfig={async () => ({ clearTagsEnabled: true, clearTags: ["comment"] })}
        saveConfig={saveConfig}
        suggestTags={suggestTags}
        loadAppInfo={async () => ({ identifier: "app", version: "1", runtime: "web", dev: false })}
      />,
    );

    const input = await screen.findByRole("combobox", { name: "Tags to clear" });
    expect(screen.getByTestId("settings-kept-tags").textContent).toContain("Description");
    fireEvent.change(input, { target: { value: "desc" } });
    const suggestion = await screen.findByRole("option", { name: "Description" });
    fireEvent.click(suggestion);

    await waitFor(() =>
      expect(saveConfig).toHaveBeenCalledWith("clearTags", ["comment", "description"]),
    );
    expect(screen.getByTestId("settings-clear-tag-list").textContent).toContain("Description");
    expect(suggestTags).toHaveBeenCalledWith("desc");
  });

  it("loads the stored schedule and marks a matching preset", async () => {
    render(
      <SettingsView
        loadBackground={async () => SETTINGS}
        loadConfig={async () => ({})}
        loadAppInfo={async () => ({ identifier: "app", version: "1.2.3", runtime: "web", dev: false })}
      />,
    );

    const preset = (await screen.findByLabelText("Schedule")) as HTMLSelectElement;
    expect(preset.value).toBe("0 0 * * *");
    expect(screen.getByText("1.2.3")).toBeTruthy();
  });

  it("offers the reference preset set including a custom expression", () => {
    // These are the reference's own values, read from its bundle: */1, */3,
    // */10, hourly, every 2 hours, daily, custom. Substituting a different
    // cadence would silently stop being the reference's preset list.
    const values = SCHEDULE_PRESETS.map((preset) => preset.value);
    expect(values).toEqual([
      "*/1 * * * *",
      "*/3 * * * *",
      "*/10 * * * *",
      "0 * * * *",
      "0 */2 * * *",
      "0 0 * * *",
      "custom",
    ]);
  });

  it("saves a preset immediately so the schedule is never left ambiguous", async () => {
    const saveBackground = vi.fn(async () => ({ ...SETTINGS, schedule: "*/5 * * * *" }));
    render(
      <SettingsView
        loadBackground={async () => SETTINGS}
        loadConfig={async () => ({})}
        saveBackground={saveBackground}
        loadAppInfo={async () => ({ identifier: "app", version: "1", runtime: "web", dev: false })}
      />,
    );

    fireEvent.change(await screen.findByLabelText("Schedule"), {
      target: { value: "*/10 * * * *" },
    });
    await waitFor(() =>
      expect(saveBackground).toHaveBeenCalledWith({ schedule: "*/10 * * * *" }),
    );
  });

  it("shows the service's reason when a crontab is unusable", async () => {
    render(
      <SettingsView
        loadBackground={async () => ({ ...SETTINGS, schedule: "*/5 * * * *" })}
        loadConfig={async () => ({})}
        saveBackground={async () => {
          throw new Error(
            "cron expression needs 5 fields (minute hour day month weekday), got 2",
          );
        }}
        loadAppInfo={async () => ({ identifier: "app", version: "1", runtime: "web", dev: false })}
      />,
    );

    fireEvent.change(await screen.findByLabelText("Schedule"), {
      target: { value: "custom" },
    });
    fireEvent.change(screen.getByLabelText("Crontab expression"), {
      target: { value: "0 0" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save schedule" }));

    // The reason appears at the field that caused it and is marked invalid, so
    // the reader does not have to hunt a banner at the top of a long page.
    const problem = await screen.findByTestId("settings-error-schedule");
    expect(problem.textContent).toContain("needs 5 fields");
    expect(screen.getByLabelText("Crontab expression").getAttribute("aria-invalid")).toBe("true");
  });

  it("adds and removes covered folders", async () => {
    const saveBackground = vi.fn(async (patch: Partial<BackgroundTaggingSettings>) => ({
      ...SETTINGS,
      ...patch,
    }));
    render(
      <SettingsView
        loadBackground={async () => SETTINGS}
        loadConfig={async () => ({})}
        saveBackground={saveBackground}
        loadRoots={async () => [{ name: "Music", path: "/music" }]}
        checkDirectory={async () => []}
        loadAppInfo={async () => ({ identifier: "app", version: "1", runtime: "web", dev: false })}
      />,
    );

    fireEvent.change(await screen.findByLabelText("Covered folders"), {
      target: { value: "/music/rock" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add folder" }));
    await waitFor(() =>
      expect(saveBackground).toHaveBeenCalledWith({ directories: ["/music/rock"] }),
    );
  });

  it("refuses a folder no library can read, instead of configuring it to never run", async () => {
    // A folder outside every served root can never be read, so accepting it
    // would leave the reader believing it is covered when it silently is not.
    const saveBackground = vi.fn(async (patch: Partial<BackgroundTaggingSettings>) => ({
      ...SETTINGS,
      ...patch,
    }));
    render(
      <SettingsView
        loadBackground={async () => SETTINGS}
        loadConfig={async () => ({})}
        saveBackground={saveBackground}
        loadRoots={async () => [{ name: "Music", path: "/music" }]}
        checkDirectory={async () => []}
        loadAppInfo={async () => ({ identifier: "app", version: "1", runtime: "web", dev: false })}
      />,
    );

    fireEvent.change(await screen.findByLabelText("Covered folders"), {
      target: { value: "/elsewhere/rock" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add folder" }));

    const problem = await screen.findByTestId("settings-error-directories");
    expect(problem.textContent).toContain("/music");
    expect(saveBackground).not.toHaveBeenCalled();
  });

  it("refuses a folder the service cannot read, rather than storing a typo", async () => {
    const saveBackground = vi.fn();
    render(
      <SettingsView
        loadBackground={async () => SETTINGS}
        loadConfig={async () => ({})}
        saveBackground={saveBackground}
        loadRoots={async () => [{ name: "Music", path: "/music" }]}
        checkDirectory={async () => {
          throw new Error("no such directory");
        }}
        loadAppInfo={async () => ({ identifier: "app", version: "1", runtime: "web", dev: false })}
      />,
    );

    fireEvent.change(await screen.findByLabelText("Covered folders"), {
      target: { value: "/music/typo" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add folder" }));

    expect((await screen.findByTestId("settings-error-directories")).textContent).toContain(
      "no such directory",
    );
    expect(saveBackground).not.toHaveBeenCalled();
  });

  it("confirms a change at the field and lets the confirmation go", async () => {
    // The old page-level "saved" message was cleared only by the *next* save, so
    // it could sit there confirming a value that had since changed.
    render(
      <SettingsView
        loadBackground={async () => SETTINGS}
        loadConfig={async () => ({ remoteLookupEnabled: false })}
        saveConfig={async () => undefined}
        confirmMs={20}
        loadAppInfo={async () => ({ identifier: "app", version: "1", runtime: "web", dev: false })}
      />,
    );

    fireEvent.click(await screen.findByLabelText(/Allow online metadata lookups/));
    expect(await screen.findByTestId("settings-saved-remoteLookupEnabled")).toBeTruthy();

    // And it expires on its own, so it cannot outlive the value it describes.
    await waitFor(() =>
      expect(screen.queryByTestId("settings-saved-remoteLookupEnabled")).toBeNull(),
    );
  });

  it("holds a free-text value as a draft until it is saved", async () => {
    // A half-typed URL is not a decision. Saving it on every keystroke would
    // store a value the reader was still writing.
    const saveConfig = vi.fn(async () => undefined);
    render(
      <SettingsView
        loadBackground={async () => SETTINGS}
        loadConfig={async () => ({ lyricsApiUrl: "https://lrclib.net" })}
        saveConfig={saveConfig}
        loadAppInfo={async () => ({ identifier: "app", version: "1", runtime: "web", dev: false })}
      />,
    );

    const input = (await screen.findByLabelText("Lyrics service URL")) as HTMLInputElement;
    expect(input.value).toBe("https://lrclib.net");

    fireEvent.change(input, { target: { value: "https://example.org" } });
    expect(saveConfig).not.toHaveBeenCalled();
    expect(screen.getByTestId("settings-dirty")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(saveConfig).toHaveBeenCalledWith("lyricsApiUrl", "https://example.org"),
    );
  });

  it("catches an unusable URL at the field without a round trip", async () => {
    // The syntax is decidable here, so asking the service would only make the
    // reader wait to learn what the page already knows.
    const saveConfig = vi.fn(async () => undefined);
    render(
      <SettingsView
        loadBackground={async () => SETTINGS}
        loadConfig={async () => ({})}
        saveConfig={saveConfig}
        loadAppInfo={async () => ({ identifier: "app", version: "1", runtime: "web", dev: false })}
      />,
    );

    fireEvent.change(await screen.findByLabelText("Lyrics service URL"), {
      target: { value: "not a url" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect((await screen.findByTestId("settings-error-lyricsApiUrl")).textContent).toContain(
      "https://",
    );
    expect(saveConfig).not.toHaveBeenCalled();
  });

  it("groups the settings so each one can be found without reading the page", async () => {
    render(
      <SettingsView
        loadBackground={async () => SETTINGS}
        loadConfig={async () => ({})}
        loadAppInfo={async () => ({ identifier: "app", version: "1", runtime: "web", dev: false })}
      />,
    );

    for (const id of ["sources", "writing", "lyrics", "automation", "assistant", "system"]) {
      const nav = await screen.findByTestId(`settings-nav-${id}`);
      // Every nav entry must point at a group that actually exists, which is the
      // whole contract of the navigation.
      const target = document.getElementById(`settings-group-${id}`);
      expect(target).toBeTruthy();
      // And clicking it must not throw, so a dead entry cannot ship.
      fireEvent.click(nav);
    }
  });

  it("states what a custom expression means, since five fields are not readable", async () => {
    render(
      <SettingsView
        loadBackground={async () => ({ ...SETTINGS, schedule: "0 3 * * *" })}
        loadConfig={async () => ({})}
        loadAppInfo={async () => ({ identifier: "app", version: "1", runtime: "web", dev: false })}
      />,
    );

    expect((await screen.findByTestId("settings-schedule-words")).textContent).toContain(
      "Every day at 03:00",
    );
  });

  it("says that no folder means every library is covered", async () => {
    render(
      <SettingsView
        loadBackground={async () => SETTINGS}
        loadConfig={async () => ({})}
        loadAppInfo={async () => ({ identifier: "app", version: "1", runtime: "web", dev: false })}
      />,
    );
    expect(await screen.findByTestId("settings-directories-empty")).toBeTruthy();
  });

  it("writes application settings through the real config keys", async () => {
    const saveConfig = vi.fn(async () => undefined);
    render(
      <SettingsView
        loadBackground={async () => SETTINGS}
        loadConfig={async () => ({ remoteLookupEnabled: false, chineseScript: "simplified" })}
        saveConfig={saveConfig}
        loadAppInfo={async () => ({ identifier: "app", version: "1", runtime: "web", dev: false })}
      />,
    );

    fireEvent.click(await screen.findByLabelText(/Allow online metadata lookups/));
    await waitFor(() => expect(saveConfig).toHaveBeenCalledWith("remoteLookupEnabled", true));

    // Clearing the script must send null, which is what "leave unchanged" means
    // to the service; an empty string would be an invalid value.
    fireEvent.change(screen.getByLabelText("Chinese script conversion"), {
      target: { value: "" },
    });
    await waitFor(() => expect(saveConfig).toHaveBeenCalledWith("chineseScript", null));
  });

  it("uses the dedicated debug-mode channel", async () => {
    const setDebugMode = vi.fn(async () => undefined);
    render(
      <SettingsView
        loadBackground={async () => SETTINGS}
        loadConfig={async () => ({ debug: false })}
        setDebugMode={setDebugMode}
        loadAppInfo={async () => ({ identifier: "app", version: "1", runtime: "web", dev: false })}
      />,
    );

    fireEvent.click(await screen.findByLabelText(/Verbose debug logging/));
    await waitFor(() => expect(setDebugMode).toHaveBeenCalledWith(true));
  });
});

describe("Workspace settings", () => {
  it("drafts changes and only applies them on confirmation", () => {
    const onApply = vi.fn();
    render(
      <WorkspaceSettingsDrawer
        detailsVisible
        visibleColumns={["cover", "fileName", "title"]}
        onApply={onApply}
        onClose={() => {}}
      />,
    );

    fireEvent.click(screen.getByLabelText(/Show file details/));
    expect(onApply).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    expect(onApply).toHaveBeenCalledWith(false, ["cover", "fileName", "title"]);
  });

  it("keeps the file-name column, which identifies the row", () => {
    render(
      <WorkspaceSettingsDrawer
        detailsVisible
        visibleColumns={["cover", "fileName"]}
        onApply={() => {}}
        onClose={() => {}}
      />,
    );
    expect(
      (screen.getByRole("button", { name: "Hide File Name" }) as HTMLButtonElement).disabled,
    ).toBe(true);
  });

  it("closes on Escape", () => {
    const onClose = vi.fn();
    render(
      <WorkspaceSettingsDrawer
        detailsVisible
        visibleColumns={["fileName"]}
        onApply={() => {}}
        onClose={onClose}
      />,
    );
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });
});

describe("table column visibility", () => {
  const track: TrackData = {
    path: "/library/01 - Song.flac",
    title: "Song",
    artist: "Artist",
    artists: ["Artist"],
    album: "Album",
    albumArtist: "Artist",
    albumArtists: ["Artist"],
    trackNumber: 1,
    trackTotal: 2,
    discNumber: 1,
    discTotal: 1,
    year: "2026",
    genre: "Rock",
    composer: "",
    comment: "",
    description: "",
    lyrics: null,
    compilation: false,
    musicbrainzTrackId: null,
    musicbrainzAlbumId: null,
    musicbrainzArtistId: null,
    discogsArtistId: null,
    discogsReleaseId: null,
    hasCover: false,
    sizeBytes: 1,
    bitrate: 1,
    sampleRate: 44100,
    codec: "flac",
    duration: 1,
    mtime: 1,
    extraTags: {},
  };

  it("hides a column's header and its cell together", () => {
    const { container } = render(
      <TrackTable
        tracks={[track]}
        selectedPaths={[]}
        onSelectTrack={() => {}}
        visibleColumns={["fileName", "title"]}
      />,
    );
    expect(screen.getByText("File Name")).toBeTruthy();
    expect(screen.queryByText("Genres")).toBeNull();
    // One header and one cell for each visible column, and nothing else.
    expect(container.querySelectorAll("thead th")).toHaveLength(2);
    expect(container.querySelectorAll("tbody td")).toHaveLength(2);
  });

  it("keeps a floor equal to the visible columns' reference widths", () => {
    const { container } = render(
      <TrackTable
        tracks={[track]}
        selectedPaths={[]}
        onSelectTrack={() => {}}
        visibleColumns={["cover", "fileName"]}
      />,
    );
    // Hiding columns lowers the floor by exactly the widths that were removed,
    // so narrow panes still scroll instead of clipping the columns that remain.
    // The measured reference widths are 80 + 174.
    const table = container.querySelector("table") as HTMLTableElement;
    expect(table.style.minWidth).toBe("254px");
  });

  it("fills its pane instead of holding a fixed pixel width", () => {
    const { container } = render(
      <TrackTable tracks={[track]} selectedPaths={[]} onSelectTrack={() => {}} />,
    );
    const table = container.querySelector("table") as HTMLTableElement;
    // A fixed pixel width is what left the table short of a wider pane.
    expect(table.style.width).toBe("");
    expect(table.style.minWidth).toBe("970px");
  });

  it("splits the table between the visible columns in the reference's proportions", () => {
    const { container } = render(
      <TrackTable
        tracks={[track]}
        selectedPaths={[]}
        onSelectTrack={() => {}}
        visibleColumns={["cover", "fileName"]}
      />,
    );
    const widths = [...container.querySelectorAll("colgroup col")].map(
      (col) => (col as HTMLTableColElement).style.width,
    );
    // 80/254 and 174/254 of the table: the shares have to cover the whole table
    // or the pane would show a gap no matter which columns are visible.
    expect(widths).toEqual(["31.4961%", "68.5039%"]);
  });

  it("still renders every measured column by default", () => {
    const { container } = render(
      <TrackTable tracks={[track]} selectedPaths={[]} onSelectTrack={() => {}} />,
    );
    expect(container.querySelectorAll("thead th")).toHaveLength(10);
    expect(container.querySelectorAll("colgroup col")).toHaveLength(10);
  });
});
