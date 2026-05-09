"""SQLite cache for normalized lookup candidates."""

from __future__ import annotations

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


class MatchCache:
    """Persistent cache for lookup candidates."""

    def __init__(self, cache_path: Path):
        self.cache_path = cache_path
        self.cache_path.parent.mkdir(parents=True, exist_ok=True)
        self._init_schema()

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

    def _connect(self) -> sqlite3.Connection:
        return sqlite3.connect(self.cache_path)
