"""Tests for dataset download pipeline internals.

Tests the torrent parsing, bencode decoding, service selection, archive
matching, and subprocess command execution — all without requiring aria2c/7z.
"""

import shutil
from pathlib import Path

import pytest

from auto_tagger.commands.dataset import (
    _archive_matches_services,
    _bdecode,
    _best_asset_for_services,
    _require_command,
    _selected_services,
    _selected_torrent_file_indices,
    _torrent_file_paths,
)
from auto_tagger.config import Settings
from auto_tagger.exceptions import ConfigError
from auto_tagger.integrations.dataset import DatasetAsset

# ── helpers ────────────────────────────────────────────────────

def _make_torrent(*files: tuple[str, int]) -> bytes:
    """Build a minimal valid bencode torrent with the given files.

    Each file is (name, size_in_bytes).
    """
    name = b"Dataset 2025"
    pieces = b"a" * 20
    data = b"d" + b"4:info" + b"d"
    data += b"4:name" + str(len(name)).encode() + b":" + name
    data += b"12:piece length" + b"i262144e"
    data += b"6:pieces" + str(len(pieces)).encode() + b":" + pieces
    if files:
        data += b"5:files" + b"l"
        for fname, size in files:
            name_bytes = fname.encode()
            data += b"d"
            data += b"6:length" + b"i" + str(size).encode() + b"e"
            data += (
                b"4:path"
                + b"l"
                + str(len(name_bytes)).encode()
                + b":"
                + name_bytes
                + b"e"
            )
            data += b"e"
        data += b"e"  # files list
    data += b"e"  # info dict
    data += b"e"  # outer dict
    return data


# ── _bdecode (bencode parser) ─────────────────────────────────

def test_bdecode_int():
    """bdecode parses integer values."""
    result, _ = _bdecode(b"i42e")
    assert result == 42


def test_bdecode_string():
    """bdecode parses byte string values."""
    result, _ = _bdecode(b"5:hello")
    assert result == b"hello"


def test_bdecode_list():
    """bdecode parses list values."""
    result, _ = _bdecode(b"li1ei2ee")
    assert result == [1, 2]


def test_bdecode_dict():
    """bdecode parses dictionary values."""
    result, _ = _bdecode(b"d3:key5:valuee")
    assert result == {b"key": b"value"}


def test_bdecode_nested():
    """bdecode parses nested file-list structures."""
    torrent = _make_torrent(("archive.7z", 1000))
    result, _ = _bdecode(torrent)
    info = result[b"info"]
    assert b"files" in info
    assert len(info[b"files"]) == 1
    assert info[b"files"][0][b"length"] == 1000
    assert info[b"files"][0][b"path"] == [b"archive.7z"]


def test_bdecode_minimal_torrent():
    """bdecode parses a minimal valid torrent structure."""
    torrent = _make_torrent(("musicbrainz_album.7z", 1000))
    result, _ = _bdecode(torrent)
    info = result[b"info"]
    assert info[b"name"] == b"Dataset 2025"
    assert len(info[b"files"]) == 1
    assert info[b"files"][0][b"path"] == [b"musicbrainz_album.7z"]


# ── _torrent_file_paths ───────────────────────────────────────

def test_torrent_file_paths_single_file(tmp_path: Path):
    """Single-file torrent returns the name."""
    torrent = _make_torrent()  # no files — uses single-file mode
    torrent_path = tmp_path / "test.torrent"
    torrent_path.write_bytes(torrent)
    paths = _torrent_file_paths(torrent_path)
    assert paths == ["Dataset 2025"]


def test_torrent_file_paths_multi_file(tmp_path: Path):
    """Multi-file torrent returns all file paths."""
    torrent = _make_torrent(
        ("musicbrainz_album.7z", 1000),
        ("spotify_album.7z", 2000),
    )
    torrent_path = tmp_path / "test.torrent"
    torrent_path.write_bytes(torrent)
    paths = _torrent_file_paths(torrent_path)
    assert paths == ["musicbrainz_album.7z", "spotify_album.7z"]


# ── _selected_torrent_file_indices ─────────────────────────────

def test_selected_torrent_file_indices_filters_by_service(tmp_path: Path):
    """Only 7z files matching requested services are selected."""
    torrent = _make_torrent(
        ("musicbrainz_album.7z", 1000),
        ("spotify_album.7z", 2000),
        ("readme.txt", 500),
    )
    torrent_path = tmp_path / "test.torrent"
    torrent_path.write_bytes(torrent)
    indices = _selected_torrent_file_indices(torrent_path, ("spotify",))
    assert indices == [2]


def test_selected_torrent_file_indices_fallback_all_7z(tmp_path: Path):
    """When no services match, all .7z files are returned."""
    torrent = _make_torrent(
        ("musicbrainz_album.7z", 1000),
        ("spotify_album.7z", 2000),
    )
    torrent_path = tmp_path / "test.torrent"
    torrent_path.write_bytes(torrent)
    indices = _selected_torrent_file_indices(torrent_path, ("deezer",))
    assert indices == [1, 2]


# ── _archive_matches_services ──────────────────────────────────

def test_archive_matches_direct_service():
    """Archive filename containing a service name matches."""
    assert _archive_matches_services(Path("musicbrainz_album.7z"), ("musicbrainz",)) is True
    assert _archive_matches_services(Path("spotify_data.7z"), ("spotify",)) is True


def test_archive_matches_generic_name():
    """Archives without service-specific names match any services (generic)."""
    assert _archive_matches_services(Path("dataset_full.7z"), ("musicbrainz",)) is True


# ── _selected_services ─────────────────────────────────────────

def test_selected_services_defaults():
    """Default services are musicbrainz when none specified."""
    settings = Settings()
    result = _selected_services(settings, ())
    assert result == ("musicbrainz",)


def test_selected_services_validates():
    """Invalid service names raise ConfigError."""
    settings = Settings()
    with pytest.raises(ConfigError, match="Unsupported"):
        _selected_services(settings, ("invalid",))


def test_selected_services_deduplicates():
    """Duplicate service entries are removed."""
    settings = Settings()
    result = _selected_services(settings, ("musicbrainz", "musicbrainz"))
    assert result == ("musicbrainz",)


# ── _best_asset_for_services ───────────────────────────────────

def test_best_asset_selects_matching():
    """Best asset is the first (newest) that covers all requested services."""
    assets = [
        DatasetAsset(
            version="02 January 2025",
            name="Dataset 02 January 2025.torrent",
            download_url="http://example.com/1",
            services=["musicbrainz", "spotify"],
        ),
        DatasetAsset(
            version="01 January 2025",
            name="Dataset 01 January 2025.torrent",
            download_url="http://example.com/2",
            services=["musicbrainz"],
        ),
    ]
    result = _best_asset_for_services(assets, ("musicbrainz", "spotify"))
    assert result is not None
    assert result.version == "02 January 2025"


def test_best_asset_returns_none_when_no_match():
    """None returned when no asset covers all requested services."""
    assets = [
        DatasetAsset(
            version="01 January 2025",
            name="test.torrent",
            download_url="http://example.com/1",
            services=["spotify"],
        ),
    ]
    result = _best_asset_for_services(assets, ("musicbrainz",))
    assert result is None


# ── _require_command ───────────────────────────────────────────

def test_require_command_found(monkeypatch):
    """Command found on PATH returns resolved path."""
    monkeypatch.setattr(shutil, "which", lambda cmd: f"/usr/bin/{cmd}")
    result = _require_command("aria2c")
    assert result == "/usr/bin/aria2c"


def test_require_command_not_found(monkeypatch):
    """Missing command raises ConfigError."""
    monkeypatch.setattr(shutil, "which", lambda cmd: None)
    with pytest.raises(ConfigError, match="Required command not found"):
        _require_command("nonexistent_cmd")
