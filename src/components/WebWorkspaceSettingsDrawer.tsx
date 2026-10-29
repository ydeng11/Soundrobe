import React, { useEffect, useRef, useState } from "react";
import { WEB_FILE_COLUMNS, type WebFileColumn } from "./FileGrid";

export interface WebWorkspaceSettingsDrawerProps {
  detailsVisible: boolean;
  visibleColumns: WebFileColumn[];
  onApply: (detailsVisible: boolean, visibleColumns: WebFileColumn[]) => void;
  onClose: () => void;
}

/** Browser-only presentation preferences for the active Workspace. */
export function WebWorkspaceSettingsDrawer({
  detailsVisible,
  visibleColumns,
  onApply,
  onClose,
}: WebWorkspaceSettingsDrawerProps) {
  const [draftDetailsVisible, setDraftDetailsVisible] = useState(detailsVisible);
  const [draftVisibleColumns, setDraftVisibleColumns] = useState(visibleColumns);
  const dialogRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    dialogRef.current?.focus();
  }, []);

  return (
    <div
      className="web-workspace-settings-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-label="Workspace settings"
        tabIndex={-1}
        className="web-workspace-settings-drawer"
        onKeyDown={(event) => {
          if (event.key === "Escape") onClose();
        }}
      >
        <header className="web-workspace-settings-header">
          <h2>Workspace settings</h2>
          <div>
            <button type="button" className="web-button-primary" onClick={() => onApply(draftDetailsVisible, draftVisibleColumns)}>Apply</button>
            <button type="button" className="web-settings-drawer-cancel" onClick={onClose}>Cancel</button>
          </div>
        </header>

        <div className="web-workspace-settings-body">
          <section aria-labelledby="workspace-display-settings">
            <p id="workspace-display-settings" className="web-workspace-settings-section">Media workspace <span>Local library</span></p>
            <div className="web-workspace-settings-row">
              <div>
                <label htmlFor="web-workspace-details" className="web-settings-label">Show file details</label>
                <p className="web-settings-hint">Display the selected track's metadata editor beside the file list.</p>
              </div>
              <button
                id="web-workspace-details"
                type="button"
                role="switch"
                aria-label="Show file details"
                aria-checked={draftDetailsVisible}
                onClick={() => setDraftDetailsVisible((visible) => !visible)}
                className={`web-workspace-settings-switch ${draftDetailsVisible ? "is-enabled" : ""}`}
              >
                <span />
              </button>
            </div>
          </section>

          <section aria-labelledby="workspace-scan-settings">
            <p id="workspace-scan-settings" className="web-workspace-settings-section">Scan scope</p>
            <p className="web-workspace-settings-note">The service controls which mounted library folders are scanned. Nested-folder scanning is not currently configurable from the browser.</p>
          </section>

          <section aria-labelledby="workspace-columns-settings">
            <p id="workspace-columns-settings" className="web-workspace-settings-section">Visible columns <span>{draftVisibleColumns.length} fields</span></p>
            <p className="web-workspace-settings-note">Choose the metadata shown in the file table.</p>
            <div className="web-workspace-columns">
              {WEB_FILE_COLUMNS.filter((column) => draftVisibleColumns.includes(column.key)).map((column) => (
                <button
                  key={column.key}
                  type="button"
                  className="web-workspace-field-chip"
                  disabled={draftVisibleColumns.length === 1}
                  aria-label={`Remove ${column.label}`}
                  onClick={() => setDraftVisibleColumns((columns) => columns.filter((key) => key !== column.key))}
                >
                  {column.label}<span aria-hidden="true">×</span>
                </button>
              ))}
            </div>
            {WEB_FILE_COLUMNS.some((column) => !draftVisibleColumns.includes(column.key)) && (
              <div className="web-workspace-column-additions" aria-label="Available file columns">
                {WEB_FILE_COLUMNS.filter((column) => !draftVisibleColumns.includes(column.key)).map((column) => (
                  <button
                    key={column.key}
                    type="button"
                    className="web-workspace-column-add"
                    onClick={() => setDraftVisibleColumns((columns) => [...columns, column.key])}
                  >
                    + {column.label}
                  </button>
                ))}
              </div>
            )}
          </section>
        </div>
      </div>
    </div>
  );
}
