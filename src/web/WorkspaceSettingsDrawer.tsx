/**
 * Workspace settings for the browser runtime.
 *
 * The reference keeps per-page presentation controls (`工作台设置`) that decide
 * what the file table shows. Soundrobe's equivalent is the details-pane toggle
 * and the visible-column set, both of which are pure browser presentation and
 * therefore stored locally rather than written to the service.
 *
 * Changes are drafted here and applied on confirmation, so an accidental click
 * cannot reshape the workspace mid-edit.
 */

import React from "react";
import { WEB_TABLE_COLUMNS, type WebTableColumnKey } from "./columns";

export interface WorkspaceSettingsDrawerProps {
  detailsVisible: boolean;
  visibleColumns: readonly WebTableColumnKey[];
  onApply: (detailsVisible: boolean, visibleColumns: WebTableColumnKey[]) => void;
  onClose: () => void;
}

export function WorkspaceSettingsDrawer({
  detailsVisible,
  visibleColumns,
  onApply,
  onClose,
}: WorkspaceSettingsDrawerProps) {
  const [draftDetails, setDraftDetails] = React.useState(detailsVisible);
  const [draftColumns, setDraftColumns] = React.useState<WebTableColumnKey[]>([
    ...visibleColumns,
  ]);
  const panelRef = React.useRef<HTMLDivElement>(null);

  React.useEffect(() => {
    panelRef.current?.focus();
  }, []);

  const hidden = WEB_TABLE_COLUMNS.filter(
    (column) => !draftColumns.includes(column.key),
  );

  return (
    <div
      className="sr-drawer__backdrop"
      data-testid="workspace-settings-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-label="Workspace settings"
        tabIndex={-1}
        className="sr-drawer"
        onKeyDown={(event) => {
          if (event.key === "Escape") onClose();
        }}
      >
        <header className="sr-toolbar">
          <h2 className="sr-section__title">Workspace settings</h2>
          <div className="sr-header__spacer" />
          <button
            type="button"
            className="sr-btn sr-btn--primary"
            onClick={() => onApply(draftDetails, draftColumns)}
          >
            Apply
          </button>
          <button type="button" className="sr-btn" onClick={onClose}>
            Cancel
          </button>
        </header>

        <section className="sr-section">
          <h3 className="sr-section__title">Media workspace</h3>
          <div className="sr-field">
            <label className="sr-field__label" htmlFor="workspace-details">
              <input
                id="workspace-details"
                type="checkbox"
                checked={draftDetails}
                onChange={(event) => setDraftDetails(event.target.checked)}
              />{" "}
              Show file details
            </label>
            <p className="sr-field__static">
              Display the selected track&rsquo;s metadata editor beside the file list.
            </p>
          </div>
        </section>

        <section className="sr-section">
          <h3 className="sr-section__title">
            Visible columns <span className="sr-cell--muted">({draftColumns.length} fields)</span>
          </h3>
          <p className="sr-field__static">Choose the metadata shown in the file table.</p>
          <ul className="sr-chips" data-testid="workspace-settings-columns">
            {WEB_TABLE_COLUMNS.filter((column) => draftColumns.includes(column.key)).map(
              (column) => (
                <li key={column.key} className="sr-chip">
                  {column.label}
                  <button
                    type="button"
                    className="sr-btn"
                    aria-label={`Hide ${column.label}`}
                    // The file name identifies the row, so it cannot be hidden.
                    disabled={column.key === "fileName"}
                    onClick={() =>
                      setDraftColumns((current) =>
                        current.filter((key) => key !== column.key),
                      )
                    }
                  >
                    Hide
                  </button>
                </li>
              ),
            )}
          </ul>
          {hidden.length > 0 && (
            <div className="sr-chips" aria-label="Available columns">
              {hidden.map((column) => (
                <button
                  key={column.key}
                  type="button"
                  className="sr-btn"
                  onClick={() => {
                    // Re-insert in reference order rather than appending, so
                    // the table keeps its measured column order.
                    const next = WEB_TABLE_COLUMNS.filter(
                      (candidate) =>
                        draftColumns.includes(candidate.key) || candidate.key === column.key,
                    ).map((candidate) => candidate.key);
                    setDraftColumns(next);
                  }}
                >
                  + {column.label}
                </button>
              ))}
            </div>
          )}
        </section>
      </div>
    </div>
  );
}
