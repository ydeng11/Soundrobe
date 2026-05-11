"""Format-specific tag mapping helpers."""

from __future__ import annotations

from base64 import b64encode
from typing import Any

from auto_tagger.core.audio import AudioFormat
from auto_tagger.core.metadata import ReplayGainTags, TrackMetadata, format_position, parse_position

MP4_FREEFORM_PREFIX = "----:com.apple.iTunes:"


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
    if audio_format in (AudioFormat.MP3, AudioFormat.WAV):
        # MP3/WAV wrapper doesn't proxy delall — unwrap to ID3
        if hasattr(tags, "tags") and hasattr(tags.tags, "delall"):
            tags = tags.tags
        _write_mp3_tags(tags, normalized)
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
        compilation=_parse_bool(_first(tags, "COMPILATION")),
        replaygain=ReplayGainTags(
            track_gain=_first(tags, "REPLAYGAIN_TRACK_GAIN"),
            track_peak=_first(tags, "REPLAYGAIN_TRACK_PEAK"),
            album_gain=_first(tags, "REPLAYGAIN_ALBUM_GAIN"),
            album_peak=_first(tags, "REPLAYGAIN_ALBUM_PEAK"),
        ),
    ).normalized()


def _write_vorbis_tags(tags: Any, metadata: TrackMetadata) -> None:
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
        compilation=_parse_bool(_first(tags, "TCMP") or _first(tags, "TXXX:COMPILATION")),
        replaygain=ReplayGainTags(
            track_gain=_first(tags, "TXXX:REPLAYGAIN_TRACK_GAIN"),
            track_peak=_first(tags, "TXXX:REPLAYGAIN_TRACK_PEAK"),
            album_gain=_first(tags, "TXXX:REPLAYGAIN_ALBUM_GAIN"),
            album_peak=_first(tags, "TXXX:REPLAYGAIN_ALBUM_PEAK"),
        ),
    ).normalized()


def _write_mp3_tags(tags: Any, metadata: TrackMetadata) -> None:
    from mutagen.id3 import TALB, TCON, TDRC, TIT2, TPE1, TPE2, TPOS, TRCK, TXXX, USLT

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
        compilation=_parse_bool(_first_raw(tags, "cpil")),
        replaygain=ReplayGainTags(
            track_gain=_first(tags, f"{MP4_FREEFORM_PREFIX}REPLAYGAIN_TRACK_GAIN"),
            track_peak=_first(tags, f"{MP4_FREEFORM_PREFIX}REPLAYGAIN_TRACK_PEAK"),
            album_gain=_first(tags, f"{MP4_FREEFORM_PREFIX}REPLAYGAIN_ALBUM_GAIN"),
            album_peak=_first(tags, f"{MP4_FREEFORM_PREFIX}REPLAYGAIN_ALBUM_PEAK"),
        ),
    ).normalized()


def _write_mp4_tags(tags: Any, metadata: TrackMetadata) -> None:
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


def embed_cover_art(audio_format: AudioFormat, tags: Any, data: bytes, mime_type: str) -> None:
    """Embed front cover art into format-specific tags."""
    if audio_format is AudioFormat.MP3:
        from mutagen.id3 import APIC

        tags.delall("APIC")
        tags.add(APIC(encoding=3, mime=mime_type, type=3, desc="Cover", data=data))
    elif audio_format is AudioFormat.M4A:
        tags["covr"] = [data]
    else:
        tags["METADATA_BLOCK_PICTURE"] = [b64encode(data).decode("ascii")]
