"""Import pg_dump SQL files directly into SQLite and provide lookup queries.

Replaces the CSV/SQL-based ETL pipeline.  Instead of building a separate
denormalised index, we import the raw PostgreSQL dumps into SQLite tables
and JOIN them at query time.
"""

from __future__ import annotations

import re
import sqlite3
from collections.abc import Iterable, Mapping
from contextlib import closing
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import opencc

from auto_tagger.integrations.candidates import (
    AlbumCandidate,
    LookupSource,
    TrackCandidate,
)

# ── SQL dump → SQLite import ────────────────────────────────────────


def import_raw_tables(
    source_dir: Path,
    db_path: Path,
    services: Iterable[str],
) -> dict[str, int]:
    """Import pg_dump *.sql files into a SQLite database.

    Reads CREATE TABLE statements to build the schema, then executes
    INSERT statements, converting PostgreSQL booleans (true/false) to
    SQLite integers (1/0).

    Returns {table_name: row_count} for each imported table.
    """
    selected = set(s.lower() for s in services)
    db_path.parent.mkdir(parents=True, exist_ok=True)

    conn = sqlite3.connect(db_path)
    conn.execute("PRAGMA journal_mode=OFF")
    conn.execute("PRAGMA synchronous=0")
    conn.execute("PRAGMA cache_size=-1000000")  # 1 GB

    counts: dict[str, int] = {}

    try:
        for sql_file in sorted(source_dir.rglob("*.sql")):
            service = _service_from_filename(sql_file)
            if service and service not in selected:
                continue

            table_name = _table_name_from_file(sql_file)
            if table_name is None:
                continue

            print(f"  Importing {sql_file.name}...")
            row_count = _import_one_file(conn, sql_file)
            counts[table_name] = row_count
            print(f"    {row_count:,} rows")

        # Create indexes for JOIN performance
        _create_lookup_indexes(conn)
        conn.commit()
    finally:
        conn.close()

    return counts


def _service_from_filename(path: Path) -> str | None:
    """Extract service name from filename like musicbrainz_release.sql."""
    name = path.stem.lower()
    for svc in ("musicbrainz", "spotify", "tidal", "deezer"):
        if name.startswith(svc):
            return svc
    return None


def _table_name_from_file(path: Path) -> str | None:
    """Extract table name from filename like musicbrainz_release.sql → musicbrainz_release."""
    name = path.stem
    # Only import files that are actual data tables (not relation/join tables
    # we don't need for lookup — skip *_artist, *_genre, *_link, *_similar, *_provider, *_externalid, *_image, *_label)
    skip_patterns = (
        "_artist.sql",           # join tables like track_artist, album_artist
        "_genre.sql",
        "_image",
        "_link.sql",
        "_similar.sql",
        "_provider.sql",
        "_externalid.sql",
        "_label.sql",
        "area.sql",
        "genre.sql",
        "label.sql",
    )
    for pat in skip_patterns:
        if name.endswith(pat):
            return None
    return name


def _import_one_file(conn: sqlite3.Connection, path: Path) -> int:
    """Import a single pg_dump .sql file into SQLite.

    Streams the file, parses CREATE TABLE for schema, then parses
    individual VALUE tuples and batch-inserts them with executemany.
    Returns total row count.
    """
    table_name: str | None = None
    columns: list[str] = []
    total_rows = 0
    batch: list[tuple[Any, ...]] = []
    BATCH_SIZE = 10000
    in_create = False
    create_lines: list[str] = []
    in_values = False
    column_count = 0

    # State for the tuple parser
    buf = ""
    depth = 0
    in_string = False
    escape_next = False

    with path.open("r", encoding="utf-8", errors="replace") as fh:
        for line in fh:
            stripped = line.strip()

            # Skip comments and config
            if not stripped or stripped.startswith("--"):
                continue
            if stripped.startswith("SET ") or stripped.startswith("SELECT "):
                continue
            if stripped.startswith("\\") or stripped.startswith("ALTER "):
                continue

            # CREATE TABLE block
            if not in_values and not in_create:
                if re.match(r'CREATE\s+TABLE\s+"public"', stripped, re.IGNORECASE):
                    in_create = True
                    create_lines = [stripped]
                    continue

            if in_create:
                create_lines.append(stripped)
                if stripped.endswith(");"):
                    full_create = " ".join(create_lines)
                    table_name = _execute_create_table(conn, full_create)
                    if table_name:
                        columns = _get_table_columns(conn, table_name)
                        column_count = len(columns)
                    in_create = False
                    create_lines = []
                continue

            # Detect start of INSERT INTO ... VALUES
            if not in_values:
                if re.match(
                    r'INSERT\s+INTO\s+"public"\."(\w+)"\s+VALUES',
                    stripped,
                    re.IGNORECASE,
                ):
                    in_values = True
                    # Check if values start on same line
                    idx = stripped.upper().find("VALUES")
                    remainder = stripped[idx + 6:].strip()
                    if remainder:
                        buf = remainder
                    else:
                        buf = ""
                continue

            # Inside VALUES block — line contains tuple fragments
            if in_values:
                buf += " " + stripped  # add space to separate line breaks

                # Parse tuples from the buffer
                buf, rows, done = _extract_tuples(buf, column_count)

                for row in rows:
                    # Convert booleans in the tuple
                    cleaned = tuple(
                        1 if v == "true" else 0 if v == "false" else v
                        for v in row
                    )
                    batch.append(cleaned)

                    if len(batch) >= BATCH_SIZE:
                        total_rows += _flush_batch(conn, table_name, batch)
                        batch.clear()

                if done:
                    in_values = False
                    buf = ""

    # Flush remaining
    if batch:
        total_rows += _flush_batch(conn, table_name, batch)

    conn.commit()
    return total_rows


def _extract_tuples(buf: str, column_count: int) -> tuple[str, list[list[Any]], bool]:
    """Extract complete tuples from a pg_dump VALUES buffer.

    Returns (remaining_buffer, list_of_parsed_tuples, done_flag).
    Each tuple is a list of string values (untyped).
    done_flag is True when we've hit the final ');'.
    """
    rows: list[list[Any]] = []
    i = 0
    n = len(buf)

    while i < n:
        # Skip whitespace and commas between tuples
        while i < n and buf[i] in (" ", "\t", "\n", ","):
            if buf[i] == "," and i > 0 and buf[i - 1] == ")":
                i += 1
                break
            i += 1
        if i >= n:
            break

        # Check for end marker
        if buf[i] == ";":
            return buf[i + 1 :], rows, True

        # Must be start of a tuple: (
        if buf[i] != "(":
            # Skip non-tuple content
            i += 1
            continue

        # Parse one tuple
        i += 1  # skip opening (
        values: list[Any] = []
        current = ""
        in_str = False

        while i < n:
            ch = buf[i]

            if in_str:
                if ch == "'" and i + 1 < n and buf[i + 1] == "'":
                    current += "'"
                    i += 2
                    continue
                elif ch == "'":
                    in_str = False
                    i += 1
                    continue
                else:
                    current += ch
                    i += 1
                    continue

            # Not in string
            if ch == "'":
                in_str = True
                i += 1
                continue
            elif ch == ",":
                values.append(current.strip())
                current = ""
                i += 1
                continue
            elif ch == ")":
                values.append(current.strip())
                # Convert NULL to None, boolean strings kept for later
                values = [
                    None if v.upper() == "NULL" else v
                    for v in values
                ]
                # Pad/truncate to expected column count
                if column_count > 0:
                    if len(values) < column_count:
                        values.extend([None] * (column_count - len(values)))
                    elif len(values) > column_count:
                        values = values[:column_count]
                rows.append(values)
                i += 1
                break
            else:
                current += ch
                i += 1

        # After parsing a tuple, check if the next non-whitespace is ;
        while i < n and buf[i] in (" ", "\t", "\n", ","):
            i += 1
        if i < n and buf[i] == ";":
            return buf[i + 1 :], rows, True

    return buf[i:], rows, False


def _flush_batch(
    conn: sqlite3.Connection,
    table_name: str | None,
    batch: list[tuple[Any, ...]],
) -> int:
    """Insert a batch of tuples into the table."""
    if not batch or not table_name:
        return 0
    placeholders = ", ".join("?" for _ in batch[0])
    sql = f'INSERT INTO "{table_name}" VALUES ({placeholders})'
    try:
        conn.executemany(sql, batch)
        return len(batch)
    except sqlite3.Error:
        return 0


def _execute_create_table(conn: sqlite3.Connection, stmt: str) -> str | None:
    """Parse and execute a pg_dump CREATE TABLE statement in SQLite."""
    m = re.match(
        r'CREATE\s+TABLE\s+"public"\."(\w+)"\s*\((.*)\);',
        stmt,
        re.DOTALL | re.IGNORECASE,
    )
    if not m:
        return None
    table_name = m.group(1)
    columns_block = m.group(2)
    columns = _parse_pg_columns(columns_block)
    col_defs = ",\n  ".join(f'"{name}"' for name, _type in columns)
    conn.execute(f'DROP TABLE IF EXISTS "{table_name}"')
    conn.execute(f'CREATE TABLE "{table_name}" (\n  {col_defs}\n)')
    return table_name


def _get_table_columns(conn: sqlite3.Connection, table_name: str) -> list[str]:
    """Get column names for a table from SQLite."""
    rows = conn.execute(f'PRAGMA table_info("{table_name}")').fetchall()
    return [r[1] for r in rows]


def _parse_pg_columns(block: str) -> list[tuple[str, str]]:
    """Parse pg_dump column definitions like 'colname' 'type' NOT NULL."""
    columns: list[tuple[str, str]] = []
    # Split by comma, but not commas inside quotes or parens
    # Simple approach: split on ', ' and handle each part
    parts = _split_column_defs(block)
    for part in parts:
        part = part.strip()
        if not part:
            continue
        # Extract quoted column name
        m = re.match(r'"(\w+)"', part)
        if m:
            columns.append((m.group(1), ""))
    return columns


def _split_column_defs(block: str) -> list[str]:
    """Split CREATE TABLE column definitions on top-level commas."""
    result: list[str] = []
    depth = 0
    current = ""
    for ch in block:
        if ch == "(":
            depth += 1
            current += ch
        elif ch == ")":
            depth -= 1
            current += ch
        elif ch == "," and depth == 0:
            result.append(current)
            current = ""
        else:
            current += ch
    if current.strip():
        result.append(current)
    return result


def _convert_booleans(text: str) -> str:
    """Replace PostgreSQL boolean literals with SQLite integers.

    Careful: only replace standalone true/false tokens, not substrings.
    """
    # Match word-boundary true/false that are NOT inside quotes
    # Simple approach: replace 'true' and 'false' when surrounded by
    # non-word characters (comma, parens, whitespace, newline, tab)
    text = re.sub(r'(?<=[\s,(\t])true(?=[\s,)\t;\n])', '1', text)
    text = re.sub(r'(?<=[\s,(\t])false(?=[\s,)\t;\n])', '0', text)
    return text


# ── normalized columns ──────────────────────────────────────────────

_ARTIST_NAME_COLUMNS: dict[str, str] = {
    "musicbrainz_artist": "name",
    "spotify_artist": "name",
    "tidal_artist": "name",
    "deezer_artist": "name",
}

_ALBUM_TITLE_COLUMNS: dict[str, str] = {
    "musicbrainz_release": "title",
    "spotify_album": "name",
    "tidal_album": "title",
    "deezer_album": "title",
}

# Map album table to its artist FK column and artist table
_ALBUM_ARTIST_JOINS: dict[str, tuple[str, str, str]] = {
    # album_table → (fk_column, artist_table, artist_id_column)
    "musicbrainz_release": ("artistid", "musicbrainz_artist", "artistid"),
    "spotify_album": ("artistid", "spotify_artist", "id"),
    "tidal_album": ("artistid", "tidal_artist", "artistid"),
    "deezer_album": ("artistid", "deezer_artist", "artistid"),
}

# Track tables and their album FK
_TRACK_TABLES: dict[str, tuple[str, str]] = {
    # track_table → (album_fk_column, album_table)
    "musicbrainz_release_track": ("releaseid", "musicbrainz_release"),
    "spotify_track": ("albumid", "spotify_album"),
    "tidal_track": ("albumid", "tidal_album"),
    "deezer_track": ("albumid", "deezer_album"),
}


def normalize_lookup_text(value: str | None) -> str:
    """Normalize text for case/punctuation-insensitive matching.

    Must match the SQL normalization in _create_lookup_indexes() exactly:
      LOWER(REPLACE(REPLACE(REPLACE(name, '.', ' '), ',', ' '), '''', ''))
    """
    if not value:
        return ""
    text = value.casefold()
    text = text.replace(".", " ").replace(",", " ").replace("'", "")
    return re.sub(r"\s+", " ", text).strip()


# ── SC/TC variant helpers ───────────────────────────────────────────


def _sc_tc_variants(text: str) -> list[str]:
    """Return Simplified Chinese and Traditional Chinese variants of text.

    Returns a deduplicated list ordered as: [simplified, traditional, original].
    If text has no Chinese characters, all variants are the same and only
    one entry is returned.
    """
    t2s = opencc.OpenCC("t2s")
    s2t = opencc.OpenCC("s2t")
    sc = t2s.convert(text)
    tc = s2t.convert(text)
    seen: set[str] = set()
    result: list[str] = []
    for v in (sc, tc, text):
        if v not in seen:
            seen.add(v)
            result.append(v)
    return result


def _lookup_variants(
    artist: str, album: str
) -> list[tuple[str, str]]:
    """Build (artist, album) pairs for both SC and TC variants.

    Yields unique pairs: (sc_artist, sc_album), (tc_artist, tc_album),
    (original_artist, original_album). Deduplicates to avoid querying
    the same pair twice.
    """
    artist_variants = _sc_tc_variants(artist)
    album_variants = _sc_tc_variants(album)
    seen: set[tuple[str, str]] = set()
    pairs: list[tuple[str, str]] = []
    # Try SC+SC, TC+TC, then original+original
    for a, b in [
        (artist_variants[0], album_variants[0]),
        (artist_variants[1] if len(artist_variants) > 1 else artist_variants[0],
         album_variants[1] if len(album_variants) > 1 else album_variants[0]),
        (artist_variants[-1], album_variants[-1]),
    ]:
        key = (a, b)
        if key not in seen:
            seen.add(key)
            pairs.append(key)
    return pairs


def _create_lookup_indexes(conn: sqlite3.Connection) -> None:
    """Create a normalized lookup table and indexes for fast search.

    Builds a denormalized 'dataset_lookup' table with pre-computed
    normalized_artist and normalized_album columns + an index for
    exact-match queries.
    """
    conn.execute("""
        CREATE TABLE IF NOT EXISTS dataset_lookup (
            service TEXT NOT NULL,
            artist TEXT NOT NULL,
            album TEXT NOT NULL,
            year TEXT,
            album_id TEXT NOT NULL,
            artist_id TEXT NOT NULL,
            normalized_artist TEXT NOT NULL,
            normalized_album TEXT NOT NULL
        )
    """)

    for album_table, (fk_col, artist_table, artist_id_col) in _ALBUM_ARTIST_JOINS.items():
        exists = conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name=?",
            (album_table,),
        ).fetchone()
        if not exists:
            continue
        artist_exists = conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name=?",
            (artist_table,),
        ).fetchone()
        if not artist_exists:
            continue

        # Check if already populated for this service
        service = album_table.split("_")[0]
        existing = conn.execute(
            "SELECT COUNT(*) FROM dataset_lookup WHERE service=?",
            (service,),
        ).fetchone()
        if existing and existing[0] > 0:
            continue

        artist_name_col = _ARTIST_NAME_COLUMNS.get(artist_table, "name")
        album_title_col = _ALBUM_TITLE_COLUMNS.get(album_table, "title")

        # Determine the date/album_id columns (varies by service)
        if service == "musicbrainz":
            date_col = "date"
            id_col = "releaseid"
            artist_id_ref = "artistid"
        else:
            date_col = "releasedate"
            id_col = "albumid"
            artist_id_ref = fk_col

        # Populate via INSERT...SELECT with JOIN
        conn.execute(f"""
            INSERT INTO dataset_lookup
                (service, artist, album, year, album_id, artist_id,
                 normalized_artist, normalized_album)
            SELECT
                '{service}',
                a."{artist_name_col}",
                al."{album_title_col}",
                al."{date_col}",
                al."{id_col}",
                a."{artist_id_col}",
                LOWER(REPLACE(REPLACE(REPLACE(
                    a."{artist_name_col}", '.', ' '), ',', ' '), '''', '')),
                LOWER(REPLACE(REPLACE(REPLACE(
                    al."{album_title_col}", '.', ' '), ',', ' '), '''', ''))
            FROM "{album_table}" al
            JOIN "{artist_table}" a ON al."{fk_col}" = a."{artist_id_col}"
            WHERE al."{album_title_col}" IS NOT NULL
              AND a."{artist_name_col}" IS NOT NULL
        """)
        conn.commit()

    # Index on normalized columns for exact-match lookup
    conn.execute("""
        CREATE INDEX IF NOT EXISTS idx_lookup_norm
        ON dataset_lookup (normalized_artist, normalized_album, service)
    """)

    # Track tables: index on album FK
    for track_table, (fk_col, _album_table) in _TRACK_TABLES.items():
        exists = conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name=?",
            (track_table,),
        ).fetchone()
        if not exists:
            continue
        try:
            conn.execute(
                f'CREATE INDEX IF NOT EXISTS "idx_{track_table}_album" '
                f'ON "{track_table}" ("{fk_col}")'
            )
        except sqlite3.OperationalError:
            pass


def _add_normalized_columns(conn: sqlite3.Connection) -> None:
    """Add normalized_artist and normalized_album columns to album tables.

    These are populated from the joined artist/album names and indexed
    for fast lookup.
    """
    for album_table, (fk_col, artist_table, artist_id_col) in _ALBUM_ARTIST_JOINS.items():
        # Check if tables exist
        exists = conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name=?",
            (album_table,),
        ).fetchone()
        if not exists:
            continue
        artist_exists = conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name=?",
            (artist_table,),
        ).fetchone()
        if not artist_exists:
            continue

        artist_name_col = _ARTIST_NAME_COLUMNS.get(artist_table, "name")
        album_title_col = _ALBUM_TITLE_COLUMNS.get(album_table, "title")

        # Add normalized columns if they don't exist
        for col in ("normalized_artist", "normalized_album"):
            try:
                conn.execute(
                    f'ALTER TABLE "{album_table}" ADD COLUMN "{col}" TEXT'
                )
            except sqlite3.OperationalError:
                pass  # column already exists

        # Populate via JOIN
        conn.execute(f"""
            UPDATE "{album_table}" SET
                "normalized_artist" = LOWER(
                    REPLACE(REPLACE(REPLACE(REPLACE(
                        (SELECT "{artist_name_col}" FROM "{artist_table}"
                         WHERE "{artist_table}"."{artist_id_col}" = "{album_table}"."{fk_col}")
                    , '.', ' '), ',', ' '), '''', ''), '-', ' ')
                ),
                "normalized_album" = LOWER(
                    REPLACE(REPLACE(REPLACE(REPLACE(
                        "{album_table}"."{album_title_col}"
                    , '.', ' '), ',', ' '), '''', ''), '-', ' ')
                )
            WHERE "normalized_artist" IS NULL
        """)

        # Create indexes
        conn.execute(f"""
            CREATE INDEX IF NOT EXISTS "idx_{album_table}_norm"
            ON "{album_table}" ("normalized_artist", "normalized_album")
        """)

    # Also add normalized columns to artist tables (for artist-only lookups)
    for artist_table, name_col in _ARTIST_NAME_COLUMNS.items():
        exists = conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name=?",
            (artist_table,),
        ).fetchone()
        if not exists:
            continue
        try:
            conn.execute(
                f'ALTER TABLE "{artist_table}" ADD COLUMN "normalized_name" TEXT'
            )
        except sqlite3.OperationalError:
            pass
        conn.execute(f"""
            UPDATE "{artist_table}" SET "normalized_name" = LOWER(
                REPLACE(REPLACE(REPLACE(REPLACE(
                    "{name_col}"
                , '.', ' '), ',', ' '), '''', ''), '-', ' ')
            )
            WHERE "normalized_name" IS NULL
        """)
        conn.execute(f"""
            CREATE INDEX IF NOT EXISTS "idx_{artist_table}_norm"
            ON "{artist_table}" ("normalized_name")
        """)


# ── lookup queries ──────────────────────────────────────────────────

def query_album(
    db_path: Path,
    artist_hint: str,
    album_hint: str,
    max_candidates: int = 5,
    services: Iterable[str] | None = None,
) -> list[AlbumCandidate]:
    """Query raw database tables for album candidates.

    Iterates configured services, queries each album + artist table,
    and returns matches as AlbumCandidate objects.
    """
    selected = [s.lower() for s in (services or [])] if services else []
    if not selected:
        selected = ["musicbrainz", "spotify", "tidal", "deezer"]

    norm_artist = normalize_lookup_text(artist_hint)
    norm_album = normalize_lookup_text(album_hint)
    if not norm_artist or not norm_album:
        return []

    candidates: list[AlbumCandidate] = []

    with closing(sqlite3.connect(db_path)) as conn:
        conn.row_factory = sqlite3.Row

        # Check if lookup table exists
        has_lookup = conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='dataset_lookup'"
        ).fetchone() is not None

        if has_lookup:
            # Fast path: use pre-computed normalized lookup table
            # Try both Simplified and Traditional Chinese variants since the
            # dataset may store either form and user input may be the other.
            variants = _lookup_variants(norm_artist, norm_album)
            where_clauses: list[str] = []
            query_params: list[str] = []
            for a, b in variants:
                where_clauses.append(
                    "(normalized_artist = ? AND normalized_album = ?)"
                )
                query_params.append(a)
                query_params.append(b)
            where_sql = " OR ".join(where_clauses) if where_clauses else "0"

            rows = conn.execute(
                f"""
                SELECT service, artist, album, year, album_id, artist_id
                FROM dataset_lookup
                WHERE ({where_sql})
                ORDER BY
                    CASE service
                        WHEN 'musicbrainz' THEN 0
                        WHEN 'spotify' THEN 1
                        WHEN 'tidal' THEN 2
                        WHEN 'deezer' THEN 3
                        ELSE 4
                    END
                LIMIT ?
                """,
                (*query_params, max_candidates),
            ).fetchall()

            for row in rows:
                service = row["service"]
                track_table = _track_table_for_service(service)
                album_table = _album_table_for_service(service)
                if album_table is None:
                    continue
                tracks = _load_tracks_by_id(conn, track_table, album_table, row["album_id"], service, row["artist"])
                if tracks is None:
                    continue
                candidates.append(AlbumCandidate(
                    artist=row["artist"],
                    artists=[row["artist"]],
                    album=row["album"],
                    album_artist=row["artist"],
                    album_artists=[row["artist"]],
                    year=row["year"],
                    musicbrainz_albumid=row["album_id"] if service == "musicbrainz" else None,
                    musicbrainz_artistid=row["artist_id"] if service == "musicbrainz" else None,
                    tracks=[
                        TrackCandidate(
                            title=t.get("title"),
                            artist=t.get("artist"),
                            artists=t.get("artists") or [],
                            track_number=t.get("track_number"),
                            track_total=len(tracks) if tracks else None,
                            disc_number=t.get("disc_number"),
                            disc_total=t.get("disc_total"),
                            musicbrainz_trackid=t.get("musicbrainz_trackid"),
                            length=t.get("length"),
                        )
                        for t in tracks
                    ],
                    source=LookupSource.DATASET,
                ))

            # If exact match returned no candidates, try progressive prefix
            # fallback to handle folder names with extra subtitle/edition info
            # (e.g. "T-TIME 新歌+精选" vs DB "T-time",
            #  "Close To 蔡健雅 Original x Tanya" vs DB "Close to 蔡健雅")
            if not candidates:
                seen_ids: set[tuple[str, str]] = set()
                album_words = norm_album.split()
                for word_count in range(len(album_words) - 1, 0, -1):
                    prefix = " ".join(album_words[:word_count])
                    if len(prefix) < 2:
                        continue
                    prefix_rows = conn.execute(
                        """
                        SELECT service, artist, album, year, album_id, artist_id
                        FROM dataset_lookup
                        WHERE normalized_artist = ?
                          AND normalized_album LIKE ? || '%'
                        ORDER BY
                            CASE service
                                WHEN 'musicbrainz' THEN 0
                                WHEN 'spotify' THEN 1
                                WHEN 'tidal' THEN 2
                                WHEN 'deezer' THEN 3
                                ELSE 4
                            END
                        LIMIT ?
                        """,
                        (norm_artist, prefix, max_candidates),
                    ).fetchall()
                    for row in prefix_rows:
                        key = (row["service"], row["album_id"])
                        if key in seen_ids:
                            continue
                        seen_ids.add(key)
                        service = row["service"]
                        track_table = _track_table_for_service(service)
                        album_table = _album_table_for_service(service)
                        if album_table is None:
                            continue
                        tracks = _load_tracks_by_id(
                            conn, track_table, album_table,
                            row["album_id"], service, row["artist"]
                        )
                        if tracks is None:
                            continue
                        candidates.append(AlbumCandidate(
                            artist=row["artist"],
                            artists=[row["artist"]],
                            album=row["album"],
                            album_artist=row["artist"],
                            album_artists=[row["artist"]],
                            year=row["year"],
                            musicbrainz_albumid=row["album_id"] if service == "musicbrainz" else None,
                            musicbrainz_artistid=row["artist_id"] if service == "musicbrainz" else None,
                            tracks=[
                                TrackCandidate(
                                    title=t.get("title"),
                                    artist=t.get("artist"),
                                    artists=t.get("artists") or [],
                                    track_number=t.get("track_number"),
                                    track_total=len(tracks) if tracks else None,
                                    disc_number=t.get("disc_number"),
                                    disc_total=t.get("disc_total"),
                                    musicbrainz_trackid=t.get("musicbrainz_trackid"),
                                    length=t.get("length"),
                                )
                                for t in tracks
                            ],
                            source=LookupSource.DATASET,
                        ))
                    if candidates:
                        break

            return candidates

        # Slow path: LIKE-based fallback for databases without lookup table
        for service in selected:
            album_table = _album_table_for_service(service)
            artist_table = _artist_table_for_service(service)
            track_table = _track_table_for_service(service)

            if album_table is None or artist_table is None:
                continue

            exists = conn.execute(
                "SELECT name FROM sqlite_master WHERE type='table' AND name=?",
                (album_table,),
            ).fetchone()
            if not exists:
                continue

            fk_col = _ALBUM_ARTIST_JOINS.get(album_table, ("", "", ""))[0]
            artist_name_col = _ARTIST_NAME_COLUMNS.get(artist_table, "name")
            album_title_col = _ALBUM_TITLE_COLUMNS.get(album_table, "title")
            artist_id_col = _ALBUM_ARTIST_JOINS.get(album_table, ("", "", ""))[2]

            like_artist = f"%{norm_artist}%"
            like_album = f"%{norm_album}%"

            rows = conn.execute(
                f"""
                SELECT a."{artist_name_col}" AS artist_name,
                       al."{album_title_col}" AS album_title,
                       al.*, a.*
                FROM "{album_table}" al
                JOIN "{artist_table}" a
                  ON al."{fk_col}" = a."{artist_id_col}"
                WHERE LOWER(REPLACE(REPLACE(REPLACE(
                        a."{artist_name_col}", '.', ' '), ',', ' '), '''', '')
                      ) LIKE ?
                  AND LOWER(REPLACE(REPLACE(REPLACE(
                        al."{album_title_col}", '.', ' '), ',', ' '), '''', '')
                      ) LIKE ?
                LIMIT ?
                """,
                (like_artist, like_album, max_candidates * 3),
            ).fetchall()

            for row in rows:
                tracks = _load_tracks(conn, track_table, album_table, row, service)
                candidates.append(_candidate_from_raw_row(row, tracks, service))

    # Sort: musicbrainz first, then others
    service_order = {"musicbrainz": 0, "spotify": 1, "tidal": 2, "deezer": 3}
    candidates.sort(key=lambda c: service_order.get(c.source.value, 4))
    return candidates[:max_candidates]


def _load_tracks_by_id(
    conn: sqlite3.Connection,
    track_table: str | None,
    album_table: str,
    album_id: str,
    service: str,
    artist_name: str,
) -> list[dict[str, Any]] | None:
    """Load tracks for an album given its ID directly (for lookup table path)."""
    if track_table is None:
        return None

    exists = conn.execute(
        "SELECT name FROM sqlite_master WHERE type='table' AND name=?",
        (track_table,),
    ).fetchone()
    if not exists:
        return None

    track_info = _TRACK_TABLES.get(track_table)
    if track_info is None:
        return None
    fk_col, _ = track_info

    if service == "musicbrainz":
        title_col = "COALESCE(title, recordingtitle)"
        num_col = "COALESCE(number, position)"
        order = "COALESCE(mediaposition, 1), COALESCE(number, position, releasetrackid)"
        track_rows = conn.execute(
            f"""
            SELECT {title_col} AS track_title,
                   {num_col} AS track_number,
                   mediaposition AS disc_number,
                   mediatrackcount AS disc_total,
                   length, recordingid AS musicbrainz_trackid
            FROM "{track_table}"
            WHERE "{fk_col}" = ?
            ORDER BY {order}
            """,
            (album_id,),
        ).fetchall()
        return [
            {
                "title": r["track_title"],
                "artist": artist_name,
                "artists": [artist_name],
                "track_number": r["track_number"],
                "disc_number": r["disc_number"],
                "disc_total": r["disc_total"],
                "musicbrainz_trackid": r["musicbrainz_trackid"],
                "length": r["length"],
            }
            for r in track_rows
            if r["track_title"]
        ]
    else:
        if service == "spotify":
            title_col = "name"
            num_col = "tracknumber"
            disc_col = "discnumber"
            dur_col = "durationms"
            dur_is_ms = True
        elif service == "tidal":
            title_col = "title"
            num_col = "tracknumber"
            disc_col = "volumenumber"
            dur_col = "duration"
            dur_is_ms = False
        else:
            title_col = "title"
            num_col = "trackposition"
            disc_col = "disknumber"
            dur_col = "duration"
            dur_is_ms = False

        order = f"COALESCE({disc_col}, 1), COALESCE({num_col}, trackid)"
        track_rows = conn.execute(
            f"""
            SELECT "{title_col}" AS track_title,
                   "{num_col}" AS track_number,
                   "{disc_col}" AS disc_number,
                   "{dur_col}" AS duration_raw
            FROM "{track_table}"
            WHERE "{fk_col}" = ?
            ORDER BY {order}
            """,
            (album_id,),
        ).fetchall()

        result = []
        for r in track_rows:
            if not r["track_title"]:
                continue
            dur = r["duration_raw"]
            length = _parse_raw_duration(dur, dur_is_ms)
            result.append({
                "title": r["track_title"],
                "artist": artist_name,
                "artists": [artist_name],
                "track_number": r["track_number"],
                "disc_number": r["disc_number"],
                "length": length,
            })
        return result


def _load_tracks(
    conn: sqlite3.Connection,
    track_table: str | None,
    album_table: str,
    album_row: sqlite3.Row,
    service: str,
) -> list[dict[str, Any]]:
    """Load tracks for an album from the track table."""
    if track_table is None:
        return []

    exists = conn.execute(
        "SELECT name FROM sqlite_master WHERE type='table' AND name=?",
        (track_table,),
    ).fetchone()
    if not exists:
        return []

    # Determine album_id column and FK
    track_info = _TRACK_TABLES.get(track_table)
    if track_info is None:
        return []
    fk_col, _ = track_info

    # Determine album_id value from the album row
    album_id_col = "releaseid" if service == "musicbrainz" else "albumid"
    album_id = album_row[album_id_col]

    # Determine track columns
    if service == "musicbrainz":
        title_col = "COALESCE(title, recordingtitle)"
        num_col = "COALESCE(number, position)"
        order = "COALESCE(mediaposition, 1), COALESCE(number, position, releasetrackid)"
        track_rows = conn.execute(
            f"""
            SELECT {title_col} AS track_title,
                   {num_col} AS track_number,
                   mediaposition AS disc_number,
                   mediatrackcount AS disc_total,
                   length, recordingid AS musicbrainz_trackid
            FROM "{track_table}"
            WHERE "{fk_col}" = ?
            ORDER BY {order}
            """,
            (album_id,),
        ).fetchall()
        return [
            {
                "title": r["track_title"],
                "artist": album_row["artist_name"],
                "artists": [album_row["artist_name"]],
                "track_number": r["track_number"],
                "disc_number": r["disc_number"],
                "disc_total": r["disc_total"],
                "musicbrainz_trackid": r["musicbrainz_trackid"],
                "length": r["length"],
            }
            for r in track_rows
            if r["track_title"]
        ]
    else:
        # Spotify / Tidal / Deezer
        if service == "spotify":
            title_col = "name"
            num_col = "tracknumber"
            disc_col = "discnumber"
            dur_col = "durationms"
            dur_is_ms = True
        elif service == "tidal":
            title_col = "title"
            num_col = "tracknumber"
            disc_col = "volumenumber"
            dur_col = "duration"
            dur_is_ms = False
        else:  # deezer
            title_col = "title"
            num_col = "trackposition"
            disc_col = "disknumber"
            dur_col = "duration"
            dur_is_ms = False

        order = f"COALESCE({disc_col}, 1), COALESCE({num_col}, trackid)"
        track_rows = conn.execute(
            f"""
            SELECT "{title_col}" AS track_title,
                   "{num_col}" AS track_number,
                   "{disc_col}" AS disc_number,
                   "{dur_col}" AS duration_raw
            FROM "{track_table}"
            WHERE "{fk_col}" = ?
            ORDER BY {order}
            """,
            (album_id,),
        ).fetchall()

        result = []
        for r in track_rows:
            if not r["track_title"]:
                continue
            dur = r["duration_raw"]
            length = _parse_raw_duration(dur, dur_is_ms)
            result.append({
                "title": r["track_title"],
                "artist": album_row["artist_name"],
                "artists": [album_row["artist_name"]],
                "track_number": r["track_number"],
                "disc_number": r["disc_number"],
                "length": length,
            })
        return result


def _candidate_from_raw_row(
    row: sqlite3.Row,
    tracks: list[dict[str, Any]],
    service: str,
) -> AlbumCandidate:
    """Convert a raw JOIN row + track list into an AlbumCandidate."""
    artist_name = row["artist_name"]
    if service == "musicbrainz":
        year = _year_from_text(row["date"])
        album_id = row["releaseid"]
        artist_id = row["artistid"]
    else:
        year = _year_from_text(row["releasedate"] if "releasedate" in row.keys() else None)
        album_id = row["albumid"] if "albumid" in row.keys() else ""
        artist_id = row["artistid"] if "artistid" in row.keys() else ""

    track_candidates = [
        TrackCandidate(
            title=t.get("title"),
            artist=t.get("artist"),
            artists=t.get("artists") or [],
            track_number=t.get("track_number"),
            track_total=len(tracks) if tracks else None,
            disc_number=t.get("disc_number"),
            disc_total=t.get("disc_total"),
            musicbrainz_trackid=t.get("musicbrainz_trackid"),
            length=t.get("length"),
        )
        for t in tracks
    ]

    return AlbumCandidate(
        artist=artist_name,
        artists=[artist_name],
        album=row["album_title"],
        album_artist=artist_name,
        album_artists=[artist_name],
        year=year,
        musicbrainz_albumid=album_id if service == "musicbrainz" else None,
        musicbrainz_artistid=artist_id if service == "musicbrainz" else None,
        tracks=track_candidates,
        source=LookupSource.DATASET,
    )


def _album_table_for_service(service: str) -> str | None:
    return {
        "musicbrainz": "musicbrainz_release",
        "spotify": "spotify_album",
        "tidal": "tidal_album",
        "deezer": "deezer_album",
    }.get(service)


def _artist_table_for_service(service: str) -> str | None:
    return {
        "musicbrainz": "musicbrainz_artist",
        "spotify": "spotify_artist",
        "tidal": "tidal_artist",
        "deezer": "deezer_artist",
    }.get(service)


def _track_table_for_service(service: str) -> str | None:
    return {
        "musicbrainz": "musicbrainz_release_track",
        "spotify": "spotify_track",
        "tidal": "tidal_track",
        "deezer": "deezer_track",
    }.get(service)


# ── helpers ─────────────────────────────────────────────────────────

def _year_from_text(value: Any) -> str | None:
    if not value:
        return None
    text = str(value).strip()
    match = re.search(r"\d{4}", text)
    return match.group(0) if match else text


def _parse_raw_duration(raw: Any, is_ms: bool) -> float | None:
    if raw is None:
        return None
    if isinstance(raw, str) and ":" in raw:
        parts = raw.strip().split(":")
        try:
            if len(parts) == 3:
                return int(parts[0]) * 3600 + int(parts[1]) * 60 + int(parts[2])
            elif len(parts) == 2:
                return int(parts[0]) * 60 + int(parts[1])
        except (ValueError, TypeError):
            pass
        return None
    try:
        val = int(raw)
        return val / 1000.0 if is_ms else float(val)
    except (TypeError, ValueError):
        return None
