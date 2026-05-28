"""Prompt builders for LLM tagging decisions."""

from __future__ import annotations

import json
from typing import Any

from auto_tagger.core.metadata import TrackMetadata
from auto_tagger.integrations.candidates import AlbumCandidate, LookupRequest

def build_selection_messages(
    request: LookupRequest,
    candidates: list[AlbumCandidate],
    max_candidates: int = 5,
) -> list[dict[str, str]]:
    """Build compact messages for candidate selection."""
    payload = {
        "artist_hint": request.artist_hint,
        "album_hint": request.album_hint,
        "track_count": len(request.tracks),
        "track_titles": [track.title for track in request.tracks if track.title],
        "candidates": [
            _candidate_summary(index, candidate)
            for index, candidate in enumerate(candidates[:max_candidates])
        ],
    }
    return [
        {
            "role": "system",
            "content": (
                "Select the best album candidate for audio tagging. "
                "Return only JSON with selected_index, confidence, and reason. "
                "Use selected_index null when all candidates are poor."
            ),
        },
        {"role": "user", "content": json.dumps(payload, ensure_ascii=False, sort_keys=True)},
    ]


def build_fallback_messages(
    request: LookupRequest,
    folder_candidate: AlbumCandidate,
    current_metadata: list[TrackMetadata],
) -> list[dict[str, str]]:
    """Build compact messages for fallback tag generation."""
    payload: dict[str, Any] = {
        "artist_hint": request.artist_hint,
        "album_hint": request.album_hint,
        "folder_candidate": _candidate_summary(0, folder_candidate),
        "current_tracks": [
            {
                "title": metadata.title,
                "artist": metadata.artist,
                "album": metadata.album,
                "track_number": metadata.track_number,
            }
            for metadata in current_metadata
        ],
    }
    return [
        {
            "role": "system",
            "content": (
                "Generate conservative fallback music tags as JSON. "
                "Do not invent MusicBrainz IDs. Leave uncertain fields empty. "
                "Return artist, artists, album, album_artist, album_artists, "
                "tracks, genre, confidence, and reason. "
                "For genre, use Discogs-style comma-separated tags "
                "(e.g. 'Electronic, House, Deep House' or 'Rock, Alternative, Indie' "
                "or 'Stage & Screen, Score, Contemporary Classical'). "
                "Only include genre if you are confident in the classification; "
                "leave null otherwise."
            ),
        },
        {"role": "user", "content": json.dumps(payload, ensure_ascii=False, sort_keys=True)},
    ]


def _candidate_summary(index: int, candidate: AlbumCandidate) -> dict[str, Any]:
    return {
        "index": index,
        "source": candidate.source.value,
        "artist": candidate.artist,
        "album": candidate.album,
        "album_artist": candidate.album_artist,
        "year": candidate.year,
        "musicbrainz_albumid": candidate.musicbrainz_albumid,
        "distance": candidate.distance,
        "tracks": [
            {
                "title": track.title,
                "track_number": track.track_number,
                "disc_number": track.disc_number,
            }
            for track in candidate.tracks[:20]
        ],
    }


def build_folder_extraction_messages(
    folder_name: str,
    parent_name: str | None = None,
) -> list[dict[str, str]]:
    """Build messages for extracting structured metadata from a folder name.

    The deterministic parser couldn't extract clean artist/album/year from
    this folder name (e.g., dot-separated ``Year.Artist.Album`` convention
    or multi-artist names). The LLM is asked to extract the fields.

    The ``parent_name`` (if provided) gives context — for CD subdirectories
    like ``Album CD1``, the parent folder ``Year.Artist.Album 3CD`` contains
    the year and artist info.
    """
    payload: dict[str, Any] = {
        "folder_name": folder_name,
        "instruction": (
            "Extract the artist, album name, and release year from this music folder name. "
            "If the folder name also indicates a disc number (e.g., CD1, Disc 2), "
            "include it. Return only JSON with these fields: "
            "artist (str), album (str), year (str or null), disc (str or null)."
        ),
    }
    if parent_name:
        payload["parent_name"] = parent_name

    return [
        {
            "role": "system",
            "content": (
                "You extract structured music metadata from folder names. "
                "Handle Chinese naming conventions like Year.Artist.Album "
                "and multi-artist folders (e.g., '2006.Artist1.Artist2.Album'). "
                "Return only valid JSON."
            ),
        },
        {"role": "user", "content": json.dumps(payload, ensure_ascii=False, sort_keys=True)},
    ]


# ── Audit prompts ─────────────────────────────────────────────────────────────


def build_audit_messages(
    album_artist_hint: str | None,
    album_hint: str | None,
    tracks: list[TrackMetadata],
    filenames: list[str],
) -> list[dict[str, str]]:
    """Build messages for LLM metadata audit.

    Audits per track: artist, title, album, album_artist, artists, path (filename).
    Includes positive and negative examples in the system prompt.
    """
    track_data = []
    for i, (meta, fn) in enumerate(zip(tracks, filenames)):
        track_data.append({
            "index": i,
            "path": fn,
            "title": meta.title or "",
            "artist": meta.artist or "",
            "album": meta.album or "",
            "album_artist": meta.album_artist or "",
            "artists": ", ".join(meta.artists) if meta.artists else "",
        })

    payload = {
        "album_folder": album_hint or "",
        "artist_folder": album_artist_hint or "",
        "tracks": track_data,
    }

    return [
        {
            "role": "system",
            "content": (
                "You audit music track metadata. The **primary principle** is that "
                "the file path must match the metadata: the album folder name should "
                "match the `album` field, the parent artist folder should match the "
                "`album_artist` field, and each filename should match its `title` field.\n\n"
                "The input provides:\n"
                "- `album_folder`: the album directory name (authoritative source for album name)\n"
                "- `artist_folder`: the parent directory name (authoritative source for album_artist)\n"
                "- `tracks[].path`: the filename (authoritative source for track title)\n"
                "- `tracks[].album`, `tracks[].album_artist`, `tracks[].title`: current metadata values\n\n"
                "Rules:\n"
                "1. 'error' means a field is clearly wrong or missing.\n"
                "2. 'warning' means the field might be wrong (typo, inconsistent capitalization, "
                "mismatched artist/album_artist convention).\n"
                "3. 'correct' means the field looks right.\n"
                "4. **Compare `tracks[].album` with `album_folder`.** If the album folder suggests "
                "a different album name than what is tagged, flag the `album` field. "
                "The folder name is authoritative.\n"
                "5. **Compare `tracks[].album_artist` with `artist_folder`.** If the parent "
                "artist directory suggests a different artist than what is tagged, "
                "flag the `album_artist` field. The folder name is authoritative.\n"
                "6. **Compare `tracks[].title` with `tracks[].path` (filename).** "
                "If the filename suggests a different title (after stripping track numbers, "
                "separators, and extensions), flag the `title` field.\n"
                "7. If artist != album_artist and artists is empty, flag artists as warning.\n"
                "8. Don't flag empty album_artist on single-artist albums.\n"
                "9. Be conservative — only flag when you have reasonable confidence.\n"
                "10. Title casing variations ('Come Together' vs 'come together') are warnings, "
                "not errors.\n"
                "11. For Chinese tracks: judge the correct character script "
                "(Simplified vs Traditional) based on the filename. The filename "
                "is the authoritative source for which script to use. If the "
                "filename uses Simplified Chinese, suggest Simplified Chinese "
                "in the title field; if it uses Traditional, suggest Traditional.\n"
                "12. **For every track with a warning or error, provide the complete "
                "corrected metadata in the `corrected` field.** Populate `corrected` "
                "with all the metadata fields that the track SHOULD have — title, "
                "artist, artists, album, album_artist, year, genre. Only include "
                "fields that are relevant (you don't need to repeat values that "
                "are already correct; the code will merge your corrected values "
                "with the existing metadata).\n"
                "13. The `suggestion` field is still used for per-field display "
                "but `corrected` is what gets written to the file.\n\n"
                "Examples of path-metadata matches (no issues):\n"
                "- album_folder='OK Computer', album='OK Computer' → correct\n"
                "- artist_folder='Radiohead', album_artist='Radiohead' → correct\n"
                "- path='01. Karma Police.flac', title='Karma Police' → correct\n"
                "\n"
                "Examples of path-metadata mismatches (with corrected metadata):\n"
                "- album_folder='OK Computer', album='OK Computer 1997' (wrong) → "
                "{ \"index\": 0, \"field\": \"album\", \"status\": \"error\", "
                "\"message\": \"Album tag 'OK Computer 1997' does not match folder name 'OK Computer'\", "
                "\"suggestion\": \"OK Computer\", "
                "\"corrected\": { \"album\": \"OK Computer\" } }\n"
                "- artist_folder='Pink Floyd', album_artist='Pink Floyd' but artist='David Gilmour' → "
                "{ \"index\": 0, \"field\": \"artist\", \"status\": \"warning\", "
                "\"message\": \"Track artist 'David Gilmour' differs from album_artist 'Pink Floyd', "
                "which matches the artist folder\", "
                "\"suggestion\": \"Pink Floyd\", "
                "\"corrected\": { \"artist\": \"Pink Floyd\" } }\n"
                "- path='01. 我爱的人.flac', title='I Love You' (English, not matching filename) → "
                "{ \"index\": 0, \"field\": \"title\", \"status\": \"error\", "
                "\"message\": \"Title 'I Love You' does not match filename '01. 我爱的人.flac'\", "
                "\"suggestion\": \"我爱的人\", "
                "\"corrected\": { \"title\": \"我爱的人\" } }\n"
                "- filename='03 - Bohemian Rhapsody.flac', title='Bohemian Rhapsody (Remastered 2011)' → "
                "{ \"index\": 0, \"field\": \"title\", \"status\": \"warning\", "
                "\"message\": \"Title 'Bohemian Rhapsody (Remastered 2011)' may have extra suffix not in "
                "filename\", "
                "\"suggestion\": \"Bohemian Rhapsody\", "
                "\"corrected\": { \"title\": \"Bohemian Rhapsody\" } }\n\n"
                "Other common patterns (preserved from existing logic):\n"
                "- artist='Beatles' (missing 'The') → "
                "{ \"index\": 0, \"field\": \"artist\", \"status\": \"warning\", "
                "\"message\": \"Artist may be missing 'The'\", "
                "\"suggestion\": \"The Beatles\" }\n"
                "- year='20' (truncated) → "
                "{ \"index\": 0, \"field\": \"year\", \"status\": \"error\", "
                "\"message\": \"Year truncated to 2 digits\", "
                "\"suggestion\": \"2020\" }\n"
                "- title is placeholder string → "
                "{ \"index\": 0, \"field\": \"title\", \"status\": \"error\", "
                "\"message\": \"Title is placeholder text, not real track name\" }\n\n"
                "Return only valid JSON. No markdown, no code fences, no extra text."
            ),
        },
        {"role": "user", "content": json.dumps(payload, ensure_ascii=False, sort_keys=True)},
    ]
