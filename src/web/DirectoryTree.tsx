/**
 * Folder tree for the browser workspace.
 *
 * The reference workspace browses folders from a tree and lists the files of the
 * selected album in the file table. An artist root is the no-album view, so its
 * loaded child-album tracks are listed together. Directory metadata remains lazy
 * outside that root-level track aggregation.
 */

import type { DirEntry } from "../shared/desktop-api";
import { isNeedsReview } from "./review";

export interface DirectoryTreeProps {
  rootLabel: string;
  rootPath: string;
  activePath: string | null;
  expanded: ReadonlySet<string>;
  childrenByPath: Readonly<Record<string, DirEntry[] | undefined>>;
  /** Albums background tagging left needing review, already normalized. */
  reviewPaths?: ReadonlySet<string>;
  onToggle: (path: string) => void;
  onNavigate: (path: string) => void;
}

export function DirectoryTree({
  rootLabel,
  rootPath,
  activePath,
  expanded,
  childrenByPath,
  reviewPaths,
  onToggle,
  onNavigate,
}: DirectoryTreeProps) {
  return (
    <div className="sr-tree" data-testid="directory-tree">
      <h2 className="sr-tree__title">Folders</h2>
      <TreeRow
        label={rootLabel}
        path={rootPath}
        depth={0}
        activePath={activePath}
        expanded={expanded}
        childrenByPath={childrenByPath}
        reviewPaths={reviewPaths}
        onToggle={onToggle}
        onNavigate={onNavigate}
      />
    </div>
  );
}

interface TreeRowProps extends Omit<DirectoryTreeProps, "rootLabel" | "rootPath"> {
  label: string;
  path: string;
  depth: number;
}

function TreeRow({
  label,
  path,
  depth,
  activePath,
  expanded,
  childrenByPath,
  reviewPaths,
  onToggle,
  onNavigate,
}: TreeRowProps) {
  const children = childrenByPath[path];
  const isExpanded = expanded.has(path);
  const isActive = activePath === path;
  const needsReview = reviewPaths ? isNeedsReview(reviewPaths, path) : false;

  return (
    <>
      <div style={{ paddingLeft: depth * 12 }}>
        <button
          type="button"
          className={`sr-tree__node ${isActive ? "is-active" : ""}`}
          aria-expanded={children ? isExpanded : undefined}
          aria-current={isActive ? "true" : undefined}
          onClick={() => {
            onNavigate(path);
            onToggle(path);
          }}
        >
          <span aria-hidden="true" className="sr-tree__twisty">
            {isExpanded ? "▾" : "▸"}
          </span>
          <span className="sr-tree__label">{label}</span>
          {needsReview && (
            <span
              className="sr-badge sr-badge--warning"
              data-testid={`tree-review-${path}`}
              title="Background tagging left this album needing review"
            >
              Review
            </span>
          )}
        </button>
      </div>

      {isExpanded &&
        (children ?? []).map((child) => (
          <TreeRow
            key={child.path}
            label={child.name}
            path={child.path}
            depth={depth + 1}
            activePath={activePath}
            expanded={expanded}
            childrenByPath={childrenByPath}
            reviewPaths={reviewPaths}
            onToggle={onToggle}
            onNavigate={onNavigate}
          />
        ))}
    </>
  );
}
