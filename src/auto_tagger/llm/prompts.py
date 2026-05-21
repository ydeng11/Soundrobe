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
