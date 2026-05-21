"""SQLite cache for normalized lookup candidates."""

from __future__ import annotations

import hashlib
import json
import sqlite3
from contextlib import closing
from datetime import datetime, timezone
from pathlib import Path

from auto_tagger.integrations.candidates import (
    AlbumCandidate,
    LookupRequest,
    candidates_from_json,
    candidates_to_json,
)

_VALID_STATUSES = frozenset({"pending", "llm_parsed", "tagged_ok", "error"})


def _sha256_hash(text: str) -> str:
    """Return stable SHA-256 hex digest of *text*."""
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def _path_hash(path: Path) -> str:
    """Stable hash of an absolute path."""
    return _sha256_hash(str(path.resolve()))


def _folder_name_hash(name: str) -> str:
    """Stable hash of a folder name (for cross-subdirectory reuse)."""
    return _sha256_hash(name.strip())


def _content_hash(album_path: Path) -> str:
    """Hash of (sorted filenames + sizes) for change detection."""
    if not album_path.is_dir():
        return ""
    entries = []
    for f in sorted(album_path.iterdir()):
        if f.is_file():
            try:
                size = f.stat().st_size
            except OSError:
                size = 0
            entries.append(f"{f.name}:{size}")
    return _sha256_hash("|".join(entries))


class MatchCache:
    """Persistent cache for lookup candidates and album processing state."""

    def __init__(self, cache_path: Path):
        self.cache_path = cache_path
        self.cache_path.parent.mkdir(parents=True, exist_ok=True)
        self._init_schema()

    # ── lookup cache (existing) ────────────────────────────────────

    def get(self, request: LookupRequest) -> list[AlbumCandidate] | None:
        """Return cached candidates for a request, if present."""
        with closing(self._connect()) as conn:
            with conn:
                row = conn.execute(
                    "SELECT response_json FROM lookup_cache WHERE query_hash = ?",
                    (request.query_hash(),),
                ).fetchone()
        if row is None:
            return None
        return candidates_from_json(row[0])

    def set(self, request: LookupRequest, candidates: list[AlbumCandidate]) -> None:
        """Store candidates for a lookup request."""
        if not candidates:
            return

        now = datetime.now(timezone.utc).isoformat()
        source = candidates[0].source.value
        with closing(self._connect()) as conn:
            with conn:
                conn.execute(
                    """
                    INSERT OR REPLACE INTO lookup_cache
                        (query_hash, query_json, response_json, created_at, source)
                    VALUES (?, ?, ?, ?, ?)
                    """,
                    (
                        request.query_hash(),
                        json.dumps(request.to_dict(), sort_keys=True),
                        candidates_to_json(candidates),
                        now,
                        source,
                    ),
                )

    # ── album state ledger ─────────────────────────────────────────

    def get_album_state(self, album_path: Path) -> dict[str, str | int | None] | None:
        """Return stored state for an album path, or None if unknown."""
        ph = _path_hash(album_path)
        with closing(self._connect()) as conn:
            with conn:
                row = conn.execute(
                    """
                    SELECT status, content_hash, folder_name_hash, llm_extraction,
                           disc_count, error, processed_at
                    FROM album_state WHERE path_hash = ?
                    """,
                    (ph,),
                ).fetchone()
        if row is None:
            return None
        return {
            "status": row[0],
            "path_hash": ph,
            "content_hash": row[1],
            "folder_name_hash": row[2],
            "llm_extraction": json.loads(row[3]) if row[3] else None,
            "disc_count": row[4],
            "error": row[5],
            "processed_at": row[6],
        }

    def set_album_state(
        self,
        album_path: Path,
        status: str,
        disc_count: int = 0,
        error: str | None = None,
    ) -> None:
        """Store or update album processing state."""
        if status not in _VALID_STATUSES:
            raise ValueError(f"Invalid album status: {status!r}. Valid: {sorted(_VALID_STATUSES)}")

        ph = _path_hash(album_path)
        ch = _content_hash(album_path)
        now = datetime.now(timezone.utc).isoformat()
        # Determine the root folder name (parent for CD subdirs)
        parent = album_path.parent
        fnh: str | None = None
        if parent and parent.name:
            fnh = _folder_name_hash(parent.name)

        with closing(self._connect()) as conn:
            with conn:
                conn.execute(
                    """
                    INSERT OR REPLACE INTO album_state
                        (path_hash, status, content_hash, folder_name_hash,
                         disc_count, error, processed_at)
                    VALUES (?, ?, ?, ?, ?, ?, ?)
                    """,
                    (ph, status, ch, fnh, disc_count, error, now),
                )

    def clear_album_state(self, album_path: Path) -> None:
        """Remove stored state for an album path."""
        ph = _path_hash(album_path)
        with closing(self._connect()) as conn:
            with conn:
                conn.execute(
                    "DELETE FROM album_state WHERE path_hash = ?",
                    (ph,),
                )

    # ── LLM folder extraction cache ───────────────────────────────

    def get_llm_extraction(self, folder_name: str) -> dict[str, str | None] | None:
        """Return cached LLM folder-name extraction, or None."""
        fnh = _folder_name_hash(folder_name)
        with closing(self._connect()) as conn:
            with conn:
                row = conn.execute(
                    "SELECT llm_extraction FROM album_state WHERE folder_name_hash = ? AND llm_extraction IS NOT NULL LIMIT 1",
                    (fnh,),
                ).fetchone()
        if row is None:
            return None
        return json.loads(row[0])

    def set_llm_extraction(self, folder_name: str, extraction: dict[str, str | None]) -> None:
        """Cache LLM folder-name extraction by folder name hash."""
        fnh = _folder_name_hash(folder_name)
        raw = json.dumps(extraction, ensure_ascii=False)
        now = datetime.now(timezone.utc).isoformat()
        with closing(self._connect()) as conn:
            with conn:
                conn.execute(
                    """
                    INSERT OR REPLACE INTO album_state
                        (path_hash, status, content_hash, folder_name_hash,
                         llm_extraction, disc_count, error, processed_at)
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                    """,
                    (f"_llm_{fnh}", "llm_parsed", "", fnh, raw, 0, None, now),
                )

    # ── schema ─────────────────────────────────────────────────────

    def _init_schema(self) -> None:
        with closing(self._connect()) as conn:
            with conn:
                conn.execute(
                    """
                    CREATE TABLE IF NOT EXISTS lookup_cache (
                        query_hash TEXT PRIMARY KEY,
                        query_json TEXT NOT NULL,
                        response_json TEXT NOT NULL,
                        created_at TEXT NOT NULL,
                        source TEXT NOT NULL
                    )
                    """
                )
                conn.execute(
                    """
                    CREATE TABLE IF NOT EXISTS album_state (
                        path_hash TEXT PRIMARY KEY,
                        status TEXT NOT NULL,
                        content_hash TEXT NOT NULL,
                        folder_name_hash TEXT,
                        llm_extraction TEXT,
                        disc_count INTEGER DEFAULT 0,
                        error TEXT,
                        processed_at TEXT
                    )
                    """
                )

    def _connect(self) -> sqlite3.Connection:
        return sqlite3.connect(self.cache_path)
