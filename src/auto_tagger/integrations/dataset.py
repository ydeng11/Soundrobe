"""Local MusicMoveArr dataset index integration."""

from __future__ import annotations

import csv
import json
import re
import sqlite3
import urllib.request
from collections.abc import Iterable, Mapping
from contextlib import closing
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from auto_tagger.integrations.candidates import (
    AlbumCandidate,
    LookupRequest,
    LookupSource,
    TrackCandidate,
)

DATASET_GITHUB_API_URL = "https://api.github.com/repos/MusicMoveArr/Datasets/contents"
SUPPORTED_SERVICES = ("musicbrainz", "tidal", "spotify", "deezer")


@dataclass(frozen=True)
class DatasetAsset:
    """Downloadable dataset metadata from the MusicMoveArr repository."""

    version: str
    name: str
    download_url: str
    services: list[str]


@dataclass(frozen=True)
class DatasetState:
    """Local dataset index setup state."""

    version: str
    services: list[str]
    source_file: str
    built_at: str
    album_rows: int
    track_rows: int

    def to_dict(self) -> dict[str, Any]:
        """Serialize state to JSON-compatible data."""
        return {
            "version": self.version,
            "services": self.services,
            "source_file": self.source_file,
            "built_at": self.built_at,
            "album_rows": self.album_rows,
            "track_rows": self.track_rows,
        }

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> DatasetState:
        """Deserialize state from JSON-compatible data."""
        return cls(
            version=str(data.get("version", "")),
            services=[str(item) for item in data.get("services", [])],
            source_file=str(data.get("source_file", "")),
            built_at=str(data.get("built_at", "")),
            album_rows=int(data.get("album_rows") or 0),
            track_rows=int(data.get("track_rows") or 0),
        )


class DatasetIndexClient:
    """Query the local SQLite dataset index."""

    def __init__(self, index_path: Path, max_candidates: int = 5):
        self.index_path = index_path
        self.max_candidates = max_candidates
        self.last_warning: str | None = None

    def lookup_album(self, request: LookupRequest) -> list[AlbumCandidate]:
        """Return dataset candidates for a lookup request."""
        self.last_warning = None
        if not self.index_path.exists():
            self.last_warning = f"Local dataset index not found at {self.index_path}"
            return []

        artist = normalize_lookup_text(request.artist_hint)
        album = normalize_lookup_text(request.album_hint)
        if not artist or not album:
            return []

        try:
            with closing(sqlite3.connect(self.index_path)) as conn:
                conn.row_factory = sqlite3.Row
                album_rows = conn.execute(
                    """
                    SELECT *
                    FROM dataset_albums
                    WHERE normalized_artist = ? AND normalized_album = ?
                    ORDER BY
                        CASE source
                            WHEN 'musicbrainz' THEN 0
                            WHEN 'spotify' THEN 1
                            WHEN 'tidal' THEN 2
                            WHEN 'deezer' THEN 3
                            ELSE 4
                        END,
                        year DESC,
                        id ASC
                    LIMIT ?
                    """,
                    (artist, album, self.max_candidates),
                ).fetchall()
                return [_candidate_from_row(conn, row) for row in album_rows]
        except sqlite3.Error as exc:
            self.last_warning = f"Local dataset index could not be queried: {exc}"
            return []


class DatasetIndexWriter:
    """Write album and track rows to a local SQLite dataset index."""

    def __init__(self, index_path: Path):
        self.index_path = index_path
        self.index_path.parent.mkdir(parents=True, exist_ok=True)
        self.conn = sqlite3.connect(index_path)
        self.album_rows = 0
        self.track_rows = 0
        self._init_schema()

    def add_album(
        self,
        *,
        source: str,
        artist: str | None,
        album: str | None,
        album_artist: str | None = None,
        artists: Iterable[str] | None = None,
        album_artists: Iterable[str] | None = None,
        year: str | None = None,
        genre: str | None = None,
        musicbrainz_albumid: str | None = None,
        musicbrainz_artistid: str | None = None,
        source_album_id: str | None = None,
        source_artist_id: str | None = None,
        tracks: Iterable[Mapping[str, Any]] | None = None,
    ) -> int:
        """Add one album and its tracks to the index."""
        display_artist = _clean_text(artist)
        display_album = _clean_text(album)
        if not display_artist or not display_album:
            return 0

        display_album_artist = _clean_text(album_artist) or display_artist
        artist_values = _clean_list(artists) or [display_artist]
        album_artist_values = _clean_list(album_artists) or [display_album_artist]

        with self.conn:
            cursor = self.conn.execute(
                """
                INSERT INTO dataset_albums (
                    source,
                    source_album_id,
                    source_artist_id,
                    artist,
                    artists_json,
                    album,
                    album_artist,
                    album_artists_json,
                    year,
                    genre,
                    musicbrainz_albumid,
                    musicbrainz_artistid,
                    normalized_artist,
                    normalized_album,
                    imported_at
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    _clean_text(source) or "unknown",
                    _clean_text(source_album_id),
                    _clean_text(source_artist_id),
                    display_artist,
                    json.dumps(artist_values),
                    display_album,
                    display_album_artist,
                    json.dumps(album_artist_values),
                    _clean_text(year),
                    _clean_text(genre),
                    _clean_text(musicbrainz_albumid),
                    _clean_text(musicbrainz_artistid),
                    normalize_lookup_text(display_artist),
                    normalize_lookup_text(display_album),
                    datetime.now(timezone.utc).isoformat(),
                ),
            )
            album_id = int(cursor.lastrowid or 0)
            self.album_rows += 1

            for track in tracks or []:
                self._add_track(album_id, track)

        return album_id

    def close(self) -> None:
        """Close the SQLite connection."""
        self.conn.close()

    def _add_track(self, album_id: int, track: Mapping[str, Any]) -> None:
        self.conn.execute(
            """
            INSERT INTO dataset_tracks (
                album_id,
                title,
                artist,
                artists_json,
                track_number,
                track_total,
                disc_number,
                disc_total,
                musicbrainz_trackid,
                length
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                album_id,
                _clean_text(track.get("title")),
                _clean_text(track.get("artist")),
                json.dumps(_clean_list(track.get("artists"))),
                _int_or_none(track.get("track_number")),
                _int_or_none(track.get("track_total")),
                _int_or_none(track.get("disc_number")),
                _int_or_none(track.get("disc_total")),
                _clean_text(track.get("musicbrainz_trackid")),
                _float_or_none(track.get("length")),
            ),
        )
        self.track_rows += 1

    def _init_schema(self) -> None:
        with self.conn:
            self.conn.execute(
                """
                CREATE TABLE IF NOT EXISTS dataset_albums (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    source TEXT NOT NULL,
                    source_album_id TEXT,
                    source_artist_id TEXT,
                    artist TEXT NOT NULL,
                    artists_json TEXT NOT NULL,
                    album TEXT NOT NULL,
                    album_artist TEXT,
                    album_artists_json TEXT NOT NULL,
                    year TEXT,
                    genre TEXT,
                    musicbrainz_albumid TEXT,
                    musicbrainz_artistid TEXT,
                    normalized_artist TEXT NOT NULL,
                    normalized_album TEXT NOT NULL,
                    imported_at TEXT NOT NULL
                )
                """
            )
            self.conn.execute(
                """
                CREATE INDEX IF NOT EXISTS idx_dataset_albums_normalized
                ON dataset_albums (normalized_artist, normalized_album)
                """
            )
            self.conn.execute(
                """
                CREATE TABLE IF NOT EXISTS dataset_tracks (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    album_id INTEGER NOT NULL,
                    title TEXT,
                    artist TEXT,
                    artists_json TEXT NOT NULL,
                    track_number INTEGER,
                    track_total INTEGER,
                    disc_number INTEGER,
                    disc_total INTEGER,
                    musicbrainz_trackid TEXT,
                    length REAL,
                    FOREIGN KEY(album_id) REFERENCES dataset_albums(id)
                )
                """
            )
            self.conn.execute(
                """
                CREATE INDEX IF NOT EXISTS idx_dataset_tracks_album_id
                ON dataset_tracks (album_id, disc_number, track_number)
                """
            )


def parse_dataset_assets(items: Iterable[Mapping[str, Any]]) -> list[DatasetAsset]:
    """Parse GitHub Contents API rows into dataset assets, newest first."""
    assets: list[DatasetAsset] = []
    for item in items:
        name = str(item.get("name", ""))
        if not name.endswith(".torrent") or "Dataset" not in name:
            continue

        services = [service for service in SUPPORTED_SERVICES if service in name.lower()]
        version = _version_from_asset_name(name)
        download_url = item.get("download_url")
        if not version or not download_url:
            continue

        assets.append(
            DatasetAsset(
                version=version,
                name=name,
                download_url=str(download_url),
                services=services,
            )
        )

    return sorted(assets, key=lambda asset: _version_sort_key(asset.version), reverse=True)


def fetch_dataset_assets(api_url: str = DATASET_GITHUB_API_URL) -> list[DatasetAsset]:
    """Fetch MusicMoveArr dataset torrent metadata from GitHub."""
    request = urllib.request.Request(
        api_url,
        headers={
            "Accept": "application/vnd.github+json",
            "User-Agent": "auto-tagger",
        },
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        payload = json.loads(response.read().decode("utf-8"))
    return parse_dataset_assets(payload)


def load_dataset_state(state_path: Path) -> DatasetState | None:
    """Load local dataset setup state if present."""
    if not state_path.exists():
        return None
    return DatasetState.from_dict(json.loads(state_path.read_text(encoding="utf-8")))


def save_dataset_state(state_path: Path, state: DatasetState) -> None:
    """Persist local dataset setup state."""
    state_path.parent.mkdir(parents=True, exist_ok=True)
    state_path.write_text(json.dumps(state.to_dict(), indent=2, sort_keys=True), encoding="utf-8")


def build_index_from_csv_tree(
    source_dir: Path,
    index_path: Path,
    services: Iterable[str],
) -> tuple[int, int]:
    """Build a SQLite lookup index from extracted CSV files.

    The upstream dataset has evolved over time, so this importer accepts common
    album column spellings instead of depending on a single exact CSV schema.
    """
    selected_services = {service.lower() for service in services}
    writer = DatasetIndexWriter(index_path)
    try:
        for csv_path in sorted(source_dir.rglob("*.csv")):
            source = _source_from_path(csv_path)
            if source is None or source not in selected_services:
                continue
            if "album" not in csv_path.name.lower() and "release" not in csv_path.name.lower():
                continue
            _import_album_csv(writer, csv_path, source)
        return writer.album_rows, writer.track_rows
    finally:
        writer.close()


def normalize_lookup_text(value: str | None) -> str:
    """Normalize lookup text for local index matching."""
    if not value:
        return ""
    text = re.sub(r"[^\w\s]", " ", value.casefold())
    return re.sub(r"\s+", " ", text).strip()


def _candidate_from_row(conn: sqlite3.Connection, row: sqlite3.Row) -> AlbumCandidate:
    track_rows = conn.execute(
        """
        SELECT *
        FROM dataset_tracks
        WHERE album_id = ?
        ORDER BY COALESCE(disc_number, 1), COALESCE(track_number, id), id
        """,
        (row["id"],),
    ).fetchall()
    return AlbumCandidate(
        artist=row["artist"],
        artists=json.loads(row["artists_json"]),
        album=row["album"],
        album_artist=row["album_artist"],
        album_artists=json.loads(row["album_artists_json"]),
        year=row["year"],
        genre=row["genre"],
        musicbrainz_albumid=row["musicbrainz_albumid"],
        musicbrainz_artistid=row["musicbrainz_artistid"],
        tracks=[_track_candidate_from_row(track_row) for track_row in track_rows],
        source=LookupSource.DATASET,
    )


def _track_candidate_from_row(row: sqlite3.Row) -> TrackCandidate:
    return TrackCandidate(
        title=row["title"],
        artist=row["artist"],
        artists=json.loads(row["artists_json"]),
        track_number=row["track_number"],
        track_total=row["track_total"],
        disc_number=row["disc_number"],
        disc_total=row["disc_total"],
        musicbrainz_trackid=row["musicbrainz_trackid"],
        length=row["length"],
    )


def _import_album_csv(writer: DatasetIndexWriter, path: Path, source: str) -> None:
    with path.open(newline="", encoding="utf-8-sig") as handle:
        reader = csv.DictReader(handle)
        for row in reader:
            writer.add_album(
                source=source,
                source_album_id=_first(row, "album_id", "release_id", "id"),
                source_artist_id=_first(row, "artist_id", "album_artist_id"),
                artist=_first(row, "artist", "artist_name", "album_artist", "albumartist"),
                album=_first(
                    row,
                    "album",
                    "album_title",
                    "release",
                    "release_title",
                    "title",
                    "name",
                ),
                album_artist=_first(row, "album_artist", "albumartist", "artist", "artist_name"),
                year=_year_from_text(_first(row, "year", "release_year", "date", "release_date")),
                genre=_first(row, "genre", "genres"),
                musicbrainz_albumid=_musicbrainz_value(
                    source,
                    row,
                    "musicbrainz_albumid",
                    "mbid",
                    "gid",
                ),
                musicbrainz_artistid=_musicbrainz_value(
                    source,
                    row,
                    "musicbrainz_artistid",
                    "artist_mbid",
                ),
            )


def _first(row: Mapping[str, Any], *keys: str) -> str | None:
    normalized = {key.lower().replace(" ", "_"): value for key, value in row.items()}
    for key in keys:
        value = normalized.get(key)
        cleaned = _clean_text(value)
        if cleaned:
            return cleaned
    return None


def _musicbrainz_value(source: str, row: Mapping[str, Any], *keys: str) -> str | None:
    if source != "musicbrainz":
        return None
    return _first(row, *keys)


def _source_from_path(path: Path) -> str | None:
    name = str(path).casefold()
    return next((service for service in SUPPORTED_SERVICES if service in name), None)


def _version_from_asset_name(name: str) -> str | None:
    match = re.search(r"Dataset\s+(.+?)(?:\.7z)?\.torrent$", name)
    return match.group(1).strip() if match else None


def _version_sort_key(version: str) -> datetime:
    for fmt in ("%d %B %Y", "%d %b %Y"):
        try:
            return datetime.strptime(version, fmt)
        except ValueError:
            continue
    return datetime.min


def _clean_text(value: Any) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _clean_list(value: Any) -> list[str]:
    if value is None:
        return []
    if isinstance(value, str):
        items = re.split(r"[;,]", value)
    else:
        items = list(value)
    return [str(item).strip() for item in items if str(item).strip()]


def _int_or_none(value: Any) -> int | None:
    if value is None:
        return None
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _float_or_none(value: Any) -> float | None:
    if value is None:
        return None
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def _year_from_text(value: str | None) -> str | None:
    if not value:
        return None
    match = re.search(r"\d{4}", value)
    return match.group(0) if match else value
