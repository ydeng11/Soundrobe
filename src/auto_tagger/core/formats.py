"""Format-specific tag mapping helpers."""

from __future__ import annotations

import logging
from base64 import b64encode
from pathlib import Path
from typing import Any

from auto_tagger.core.audio import AudioFormat
from auto_tagger.core.metadata import ReplayGainTags, TrackMetadata, format_position, parse_position

LOG = logging.getLogger(__name__)

MP4_FREEFORM_PREFIX = "----:com.apple.iTunes:"

# Tag names that are known junk/spam and should be stripped when writing tags.
JUNK_TAG_NAMES: frozenset[str] = frozenset({"description", "comment", "c"})


def read_tags(audio_format: AudioFormat, tags: Any) -> TrackMetadata:
    """Read format-specific tags into normalized metadata."""
    if audio_format in (AudioFormat.MP3, AudioFormat.WAV):
        return _read_mp3_tags(tags)
    if audio_format is AudioFormat.M4A:
        return _read_mp4_tags(tags)
    return _read_vorbis_tags(tags)


def write_tags(audio_format: AudioFormat, tags: Any, metadata: TrackMetadata) -> None:
    """Write normalized metadata to a format-specific tag object."""
    normalized = metadata.normalized()
    save_path: Path | None = None
    if audio_format in (AudioFormat.MP3, AudioFormat.WAV):
        # MP3/WAV wrapper doesn't proxy delall — unwrap to ID3
        if hasattr(tags, "tags"):
            # Grab the file path from the outer mutagen wrapper *before*
            # unwrapping, because _WaveID3 may have a None filename when
            # created fresh via add_tags().
            save_path = Path(tags.filename) if tags.filename else None

            if tags.tags is None and hasattr(tags, "add_tags"):
                tags.add_tags()
            if tags.tags is not None and hasattr(tags.tags, "delall"):
                tags = tags.tags
        _write_mp3_tags(tags, normalized)

        # Strip legacy LIST/INFO chunks from WAV files *before* the caller's
        # save(), so that when mutagen re-writes the file it never serialises
        # the LIST chunk back.  The stale LIST metadata shows as "unsupported
        # data" in tag editors like mp3tag.
        if audio_format is AudioFormat.WAV and save_path is not None:
            strip_wav_list_chunks(save_path)
    elif audio_format is AudioFormat.M4A:
        _write_mp4_tags(tags, normalized)
    else:
        _write_vorbis_tags(tags, normalized)


def _read_vorbis_tags(tags: Any) -> TrackMetadata:
    track_number, track_total = _parse_split_or_total(tags, "TRACKNUMBER", "TOTALTRACKS")
    disc_number, disc_total = _parse_split_or_total(tags, "DISCNUMBER", "TOTALDISCS")

    return TrackMetadata(
        title=_first(tags, "TITLE"),
        artist=_first(tags, "ARTIST"),
        artists=_values(tags, "ARTISTS"),
        album=_first(tags, "ALBUM"),
        album_artist=_first(tags, "ALBUMARTIST"),
        album_artists=_values(tags, "ALBUMARTISTS"),
        track_number=track_number,
        track_total=track_total,
        disc_number=disc_number,
        disc_total=disc_total,
        year=_first(tags, "DATE") or _first(tags, "YEAR"),
        genre=_first(tags, "GENRE"),
        musicbrainz_trackid=_first(tags, "MUSICBRAINZ_TRACKID"),
        musicbrainz_albumid=_first(tags, "MUSICBRAINZ_ALBUMID"),
        musicbrainz_artistid=_first(tags, "MUSICBRAINZ_ARTISTID"),
        lyrics=_first(tags, "LYRICS") or _first(tags, "UNSYNCEDLYRICS"),
        composer=_first(tags, "COMPOSER"),
        compilation=_parse_bool(_first(tags, "COMPILATION")),
        replaygain=ReplayGainTags(
            track_gain=_first(tags, "REPLAYGAIN_TRACK_GAIN"),
            track_peak=_first(tags, "REPLAYGAIN_TRACK_PEAK"),
            album_gain=_first(tags, "REPLAYGAIN_ALBUM_GAIN"),
            album_peak=_first(tags, "REPLAYGAIN_ALBUM_PEAK"),
        ),
    ).normalized()


def _write_vorbis_tags(tags: Any, metadata: TrackMetadata) -> None:
    _strip_junk_dict_tags(tags)
    _set_list(tags, "TITLE", _one(metadata.title))
    _set_list(tags, "ARTIST", _one(metadata.artist))
    _set_list(tags, "ARTISTS", metadata.artists)
    _set_list(tags, "ALBUM", _one(metadata.album))
    _set_list(tags, "ALBUMARTIST", _one(metadata.album_artist))
    _set_list(tags, "ALBUMARTISTS", metadata.album_artists)
    _set_list(tags, "TRACKNUMBER", _one(format_position(metadata.track_number)))
    _set_list(tags, "TOTALTRACKS", _one_int(metadata.track_total))
    _set_list(tags, "DISCNUMBER", _one(format_position(metadata.disc_number)))
    _set_list(tags, "TOTALDISCS", _one_int(metadata.disc_total))
    _set_list(tags, "DATE", _one(metadata.year))
    _set_list(tags, "GENRE", _one(metadata.genre))
    _set_list(tags, "MUSICBRAINZ_TRACKID", _one(metadata.musicbrainz_trackid))
    _set_list(tags, "MUSICBRAINZ_ALBUMID", _one(metadata.musicbrainz_albumid))
    _set_list(tags, "MUSICBRAINZ_ARTISTID", _one(metadata.musicbrainz_artistid))
    _set_list(tags, "LYRICS", _one(metadata.lyrics))
    _set_list(tags, "COMPOSER", _one(metadata.composer))
    _set_list(tags, "COMPILATION", _one_bool(metadata.compilation))
    _set_list(tags, "REPLAYGAIN_TRACK_GAIN", _one(metadata.replaygain.track_gain))
    _set_list(tags, "REPLAYGAIN_TRACK_PEAK", _one(metadata.replaygain.track_peak))
    _set_list(tags, "REPLAYGAIN_ALBUM_GAIN", _one(metadata.replaygain.album_gain))
    _set_list(tags, "REPLAYGAIN_ALBUM_PEAK", _one(metadata.replaygain.album_peak))


def _read_mp3_tags(tags: Any) -> TrackMetadata:
    track_number, track_total = parse_position(_first(tags, "TRCK"))
    disc_number, disc_total = parse_position(_first(tags, "TPOS"))

    return TrackMetadata(
        title=_first(tags, "TIT2"),
        artist=_first(tags, "TPE1"),
        artists=_values(tags, "TXXX:ARTISTS"),
        album=_first(tags, "TALB"),
        album_artist=_first(tags, "TPE2"),
        album_artists=_values(tags, "TXXX:ALBUMARTISTS"),
        track_number=track_number,
        track_total=track_total,
        disc_number=disc_number,
        disc_total=disc_total,
        year=_first(tags, "TDRC") or _first(tags, "TYER"),
        genre=_first(tags, "TCON"),
        musicbrainz_trackid=_first(tags, "TXXX:MusicBrainz Track Id"),
        musicbrainz_albumid=_first(tags, "TXXX:MusicBrainz Album Id"),
        musicbrainz_artistid=_first(tags, "TXXX:MusicBrainz Artist Id"),
        lyrics=_first(tags, "USLT::eng"),
        composer=_first(tags, "TCOM"),
        compilation=_parse_bool(_first(tags, "TCMP") or _first(tags, "TXXX:COMPILATION")),
        replaygain=ReplayGainTags(
            track_gain=_first(tags, "TXXX:REPLAYGAIN_TRACK_GAIN"),
            track_peak=_first(tags, "TXXX:REPLAYGAIN_TRACK_PEAK"),
            album_gain=_first(tags, "TXXX:REPLAYGAIN_ALBUM_GAIN"),
            album_peak=_first(tags, "TXXX:REPLAYGAIN_ALBUM_PEAK"),
        ),
    ).normalized()


def _write_mp3_tags(tags: Any, metadata: TrackMetadata) -> None:
    _strip_junk_mp3(tags)
    from mutagen.id3 import TALB, TCOM, TCON, TDRC, TIT2, TPE1, TPE2, TPOS, TRCK, TXXX, USLT

    _set_id3_frame(tags, "TIT2", TIT2(encoding=3, text=_one(metadata.title)))
    _set_id3_frame(tags, "TPE1", TPE1(encoding=3, text=_one(metadata.artist)))
    _set_id3_frame(tags, "TALB", TALB(encoding=3, text=_one(metadata.album)))
    _set_id3_frame(tags, "TPE2", TPE2(encoding=3, text=_one(metadata.album_artist)))
    track_position = format_position(metadata.track_number, metadata.track_total)
    disc_position = format_position(metadata.disc_number, metadata.disc_total)
    _set_id3_frame(tags, "TRCK", TRCK(encoding=3, text=_one(track_position)))
    _set_id3_frame(tags, "TPOS", TPOS(encoding=3, text=_one(disc_position)))
    _set_id3_frame(tags, "TDRC", TDRC(encoding=3, text=_one(metadata.year)))
    _set_id3_frame(tags, "TCON", TCON(encoding=3, text=_one(metadata.genre)))
    _set_id3_frame(tags, "TCOM", TCOM(encoding=3, text=_one(metadata.composer)))
    _set_txxx(tags, TXXX, "ARTISTS", metadata.artists)
    _set_txxx(tags, TXXX, "ALBUMARTISTS", metadata.album_artists)
    _set_txxx(tags, TXXX, "MusicBrainz Track Id", _one(metadata.musicbrainz_trackid))
    _set_txxx(tags, TXXX, "MusicBrainz Album Id", _one(metadata.musicbrainz_albumid))
    _set_txxx(tags, TXXX, "MusicBrainz Artist Id", _one(metadata.musicbrainz_artistid))
    _set_uslt(tags, USLT, metadata.lyrics)
    _set_txxx(tags, TXXX, "COMPILATION", _one_bool(metadata.compilation))
    _set_txxx(tags, TXXX, "REPLAYGAIN_TRACK_GAIN", _one(metadata.replaygain.track_gain))
    _set_txxx(tags, TXXX, "REPLAYGAIN_TRACK_PEAK", _one(metadata.replaygain.track_peak))
    _set_txxx(tags, TXXX, "REPLAYGAIN_ALBUM_GAIN", _one(metadata.replaygain.album_gain))
    _set_txxx(tags, TXXX, "REPLAYGAIN_ALBUM_PEAK", _one(metadata.replaygain.album_peak))


def _read_mp4_tags(tags: Any) -> TrackMetadata:
    track_number, track_total = _parse_mp4_position(_first_raw(tags, "trkn"))
    disc_number, disc_total = _parse_mp4_position(_first_raw(tags, "disk"))

    return TrackMetadata(
        title=_first(tags, "©nam"),
        artist=_first(tags, "©art"),
        artists=_values(tags, f"{MP4_FREEFORM_PREFIX}ARTISTS"),
        album=_first(tags, "©alb"),
        album_artist=_first(tags, "aART"),
        album_artists=_values(tags, f"{MP4_FREEFORM_PREFIX}ALBUMARTISTS"),
        track_number=track_number,
        track_total=track_total,
        disc_number=disc_number,
        disc_total=disc_total,
        year=_first(tags, "©day"),
        genre=_first(tags, "©gen"),
        musicbrainz_trackid=_first(tags, f"{MP4_FREEFORM_PREFIX}MUSICBRAINZ_TRACKID"),
        musicbrainz_albumid=_first(tags, f"{MP4_FREEFORM_PREFIX}MUSICBRAINZ_ALBUMID"),
        musicbrainz_artistid=_first(tags, f"{MP4_FREEFORM_PREFIX}MUSICBRAINZ_ARTISTID"),
        lyrics=_first(tags, "©lyr"),
        composer=_first(tags, "©com") or _first(tags, f"{MP4_FREEFORM_PREFIX}COMPOSER"),
        compilation=_parse_bool(_first_raw(tags, "cpil")),
        replaygain=ReplayGainTags(
            track_gain=_first(tags, f"{MP4_FREEFORM_PREFIX}REPLAYGAIN_TRACK_GAIN"),
            track_peak=_first(tags, f"{MP4_FREEFORM_PREFIX}REPLAYGAIN_TRACK_PEAK"),
            album_gain=_first(tags, f"{MP4_FREEFORM_PREFIX}REPLAYGAIN_ALBUM_GAIN"),
            album_peak=_first(tags, f"{MP4_FREEFORM_PREFIX}REPLAYGAIN_ALBUM_PEAK"),
        ),
    ).normalized()


def _write_mp4_tags(tags: Any, metadata: TrackMetadata) -> None:
    _strip_junk_dict_tags(tags)
    _set_list(tags, "©nam", _one(metadata.title))
    _set_list(tags, "©art", _one(metadata.artist))
    _set_list(tags, "©alb", _one(metadata.album))
    _set_list(tags, "aART", _one(metadata.album_artist))
    _set_mp4_position(tags, "trkn", metadata.track_number, metadata.track_total)
    _set_mp4_position(tags, "disk", metadata.disc_number, metadata.disc_total)
    _set_list(tags, "©day", _one(metadata.year))
    _set_list(tags, "©gen", _one(metadata.genre))
    _set_mp4_freeform(tags, "ARTISTS", metadata.artists)
    _set_mp4_freeform(tags, "ALBUMARTISTS", metadata.album_artists)
    _set_mp4_freeform(tags, "MUSICBRAINZ_TRACKID", _one(metadata.musicbrainz_trackid))
    _set_mp4_freeform(tags, "MUSICBRAINZ_ALBUMID", _one(metadata.musicbrainz_albumid))
    _set_mp4_freeform(tags, "MUSICBRAINZ_ARTISTID", _one(metadata.musicbrainz_artistid))
    _set_list(tags, "©lyr", _one(metadata.lyrics))
    _set_list(tags, "©com", _one(metadata.composer))
    if metadata.compilation is not None:
        tags["cpil"] = [bool(metadata.compilation)]
    _set_mp4_freeform(tags, "REPLAYGAIN_TRACK_GAIN", _one(metadata.replaygain.track_gain))
    _set_mp4_freeform(tags, "REPLAYGAIN_TRACK_PEAK", _one(metadata.replaygain.track_peak))
    _set_mp4_freeform(tags, "REPLAYGAIN_ALBUM_GAIN", _one(metadata.replaygain.album_gain))
    _set_mp4_freeform(tags, "REPLAYGAIN_ALBUM_PEAK", _one(metadata.replaygain.album_peak))


def _values(tags: Any, key: str) -> list[str]:
    return [value for value in _text_values(_get(tags, key)) if value]


def _first(tags: Any, key: str) -> str | None:
    values = _values(tags, key)
    return values[0] if values else None


def _first_raw(tags: Any, key: str) -> Any:
    value = _get(tags, key)
    if isinstance(value, (list, tuple)) and value:
        return value[0]
    return value


def _get(tags: Any, key: str) -> Any:
    if tags is None:
        return None
    getter = getattr(tags, "get", None)
    if getter is not None:
        return getter(key)
    return None


def _text_values(value: Any) -> list[str]:
    if value is None:
        return []
    if isinstance(value, bytes):
        return [value.decode("utf-8", errors="replace").strip()]
    if isinstance(value, str):
        return [value.strip()]
    if isinstance(value, (list, tuple)):
        values: list[str] = []
        for item in value:
            values.extend(_text_values(item))
        return values
    if hasattr(value, "text"):
        return _text_values(value.text)
    return [str(value).strip()]


def _parse_split_or_total(
    tags: Any,
    number_key: str,
    total_key: str,
) -> tuple[int | None, int | None]:
    number, embedded_total = parse_position(_first(tags, number_key))
    total = _parse_total(_first(tags, total_key)) or embedded_total
    return number, total


def _parse_total(value: str | None) -> int | None:
    if value is None:
        return None
    parsed, _ = parse_position(value)
    return parsed


def _parse_mp4_position(value: Any) -> tuple[int | None, int | None]:
    if value is None:
        return None, None
    if isinstance(value, tuple):
        current = value[0] if len(value) > 0 else None
        total = value[1] if len(value) > 1 else None
        return current or None, total or None
    return parse_position(value)


def _one(value: str | None) -> list[str]:
    return [value] if value else []


def _one_int(value: int | None) -> list[str]:
    return [str(value)] if value is not None else []


def _one_bool(value: bool | None) -> list[str]:
    return ["1"] if value else []


def _parse_bool(value: Any) -> bool | None:
    if isinstance(value, bool):
        return value
    if isinstance(value, (list, tuple)) and value:
        return _parse_bool(value[0])
    if value is None:
        return None
    return str(value).strip().lower() in {"1", "true", "yes"}


def _set_list(tags: Any, key: str, values: list[str]) -> None:
    if values:
        tags[key] = values


def _set_id3_frame(tags: Any, key: str, frame: Any) -> None:
    if getattr(frame, "text", None):
        tags.delall(key)
        tags.add(frame)


def _set_txxx(tags: Any, frame_type: Any, desc: str, values: list[str]) -> None:
    if values:
        tags.delall(f"TXXX:{desc}")
        tags.add(frame_type(encoding=3, desc=desc, text=values))


def _set_uslt(tags: Any, frame_type: Any, lyrics: str | None) -> None:
    if lyrics:
        tags.delall("USLT")
        tags.add(frame_type(encoding=3, lang="eng", desc="", text=lyrics))


def _set_mp4_position(tags: Any, key: str, current: int | None, total: int | None) -> None:
    if current is not None:
        tags[key] = [(current, total or 0)]


def _set_mp4_freeform(tags: Any, name: str, values: list[str]) -> None:
    if not values:
        return
    from mutagen.mp4 import MP4FreeForm

    tags[f"{MP4_FREEFORM_PREFIX}{name}"] = [MP4FreeForm(value.encode("utf-8")) for value in values]


def _strip_junk_dict_tags(tags: Any) -> None:
    """Remove junk tags from dict-style tag containers (Vorbis, MP4)."""
    for junk_key in JUNK_TAG_NAMES:
        if junk_key in tags:
            try:
                del tags[junk_key]
            except (KeyError, TypeError):
                pass


def _strip_junk_mp3(tags: Any) -> None:
    """Remove junk ID3 frames before writing.

    The caller (write_tags) already unwraps WAV/ID3 wrappers,
    so tags is the raw ID3 tags object here.
    """
    for junk_key in JUNK_TAG_NAMES:
        try:
            tags.delall(junk_key)
        except AttributeError:
            pass


def strip_wav_list_chunks(path: Path) -> bool:
    """Remove all LIST/INFO chunks from a WAV file.

    WAV files can contain both standard RIFF LIST/INFO metadata
    (sub-chunks like IART for artist, IPRD for album, etc.) and
    non-standard embedded ID3 ``id3 `` chunks.  The LIST metadata
    often carries stale or conflicting values that cause tag editors
    such as mp3tag to display "unsupported data".

    This function physically removes every LIST chunk from the file.
    It is safe to call on files with no LIST chunks (returns False).

    Returns:
        True if at least one LIST chunk was removed.

    Raises:
        TaggingError if the file is not a valid WAV file.
    """
    with open(path, "rb") as f:
        data = bytearray(f.read())

    if len(data) < 12 or data[0:4] != b"RIFF" or data[8:12] != b"WAVE":
        from auto_tagger.exceptions import TaggingError

        raise TaggingError(f"Not a valid WAV file: {path}")

    # Scan all chunks, copy everything except LIST chunks.
    cleaned = bytearray()
    cleaned.extend(data[0:12])  # RIFF marker + size placeholder + WAVE ID
    pos = 12
    removed = False

    while pos < len(data) - 8:
        chunk_id = data[pos : pos + 4]
        chunk_size = int.from_bytes(data[pos + 4 : pos + 8], "little")

        # Protect against bogus sizes (e.g. trailing garbage in the file)
        remaining = len(data) - (pos + 8)
        if chunk_size > remaining:
            LOG.warning(
                "Truncating chunk %r in %s: declared %d bytes, %d available",
                chunk_id,
                path,
                chunk_size,
                remaining,
            )
            break  # treat remaining bytes as dead data, not a valid chunk

        if chunk_id == b"LIST":
            removed = True
            pos += 8 + chunk_size
            if chunk_size % 2:
                pos += 1  # RIFF padding byte
            continue

        chunk_end = pos + 8 + chunk_size
        if chunk_size % 2:
            chunk_end += 1  # RIFF padding byte
        cleaned.extend(data[pos : min(chunk_end, len(data))])
        pos = chunk_end

    # Update the RIFF size field.
    new_size = len(cleaned) - 8
    cleaned[4:8] = new_size.to_bytes(4, "little")

    with open(path, "wb") as f:
        f.write(cleaned)

    if removed:
        LOG.info("Stripped LIST chunk(s) from %s", path)
    return removed


def embed_cover_art(audio_format: AudioFormat, tags: Any, data: bytes, mime_type: str) -> None:
    """Embed front cover art into format-specific tags."""
    if audio_format in (AudioFormat.MP3, AudioFormat.WAV):
        from mutagen.id3 import APIC

        # WAV uses _WaveID3 wrapper — unwrap if needed
        if hasattr(tags, "tags") and hasattr(tags.tags, "delall"):
            real_tags = tags.tags
        else:
            real_tags = tags

        real_tags.delall("APIC")
        real_tags.add(APIC(encoding=3, mime=mime_type, type=3, desc="Cover", data=data))
    elif audio_format is AudioFormat.M4A:
        tags["covr"] = [data]
    else:
        # FLAC / Ogg Vorbis / similar — prefer FLAC-native Picture API
        from mutagen.flac import Picture

        pic = Picture()
        pic.type = 3  # Front cover
        pic.mime = mime_type
        pic.desc = "Cover"
        if data:
            pic.data = data

        if hasattr(tags, "clear_pictures") and hasattr(tags, "add_picture"):
            tags.clear_pictures()
            tags.add_picture(pic)
        else:
            tags["METADATA_BLOCK_PICTURE"] = [b64encode(data).decode("ascii")]
