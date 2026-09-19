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
  it("carries the operation log as a section rather than a peer destination", async () => {
    // The log answers the same question this view does - what has the background
    // tagger been doing - so it lives here. If this section disappears, operations
    // become unreachable, because they no longer have a navigation entry.
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

    const section = await screen.findByTestId("operation-log");
    // Rendered with the view's own section heading, not a second view heading.
    expect(within(section).getByRole("heading", { name: "Operations" })).toBeTruthy();
    const table = within(section).getByTestId("log-table");
    expect(within(table).getByText("Running")).toBeTruthy();
    expect(within(table).getByText("Failed")).toBeTruthy();
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

    expect(await screen.findByText("Needs tagging")).toBeTruthy();
    expect(screen.getByText("/library/Album Two")).toBeTruthy();
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

    expect(await screen.findByText(/hello/)).toBeTruthy();
    fireEvent.change(screen.getByLabelText("Source"), { target: { value: "auto-tag" } });
    expect(await screen.findByText("verbose")).toBeTruthy();
    expect(readLog).toHaveBeenCalledWith("auto-tag");
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
    expect(await screen.findByTestId("monitoring-log-empty")).toBeTruthy();
  });
});

describe("Settings", () => {
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

    expect(await screen.findByTestId("settings-error")).toBeTruthy();
    expect(screen.getByText(/needs 5 fields/)).toBeTruthy();
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
