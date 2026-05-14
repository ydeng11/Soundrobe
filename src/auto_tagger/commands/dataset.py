"""Dataset setup and status command implementation."""

from __future__ import annotations

import shutil
import subprocess
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from auto_tagger.config import Settings
from auto_tagger.exceptions import ConfigError
from auto_tagger.integrations.dataset import (
    DatasetAsset,
    DatasetState,
    build_index_from_csv_tree,
    fetch_dataset_assets,
    load_dataset_state,
    save_dataset_state,
)
from auto_tagger.integrations.dataset_raw import import_raw_tables
from auto_tagger.utils import console, print_info, print_success, print_table, print_warning

SUPPORTED_SERVICES = ("musicbrainz", "spotify", "tidal", "deezer")


def execute_status(settings: Settings) -> None:
    """Show local dataset setup status."""
    state = load_dataset_state(settings.dataset_state_path)
    installed = settings.dataset_index_path.exists() and state is not None

    if installed:
        print_success("Local dataset index installed")
    else:
        print_warning("Local dataset index not installed")
    print(f"SQLite index: {settings.dataset_index_path}")
    print(f"State file: {settings.dataset_state_path}")

    rows: list[list[Any]] = [
        ["Status", "installed" if installed else "not installed"],
        ["Data directory", str(settings.data_dir)],
        ["Downloads", str(settings.dataset_downloads_dir)],
        ["Staging", str(settings.dataset_staging_dir)],
        ["SQLite index", str(settings.dataset_index_path)],
        ["State file", str(settings.dataset_state_path)],
    ]
    if state is not None:
        rows.extend(
            [
                ["Version", state.version],
                ["Services", ", ".join(state.services)],
                ["Built at", state.built_at],
                ["Album rows", state.album_rows],
                ["Track rows", state.track_rows],
            ]
        )

    print_table("Dataset status", ["Key", "Value"], rows)


def execute_setup(
    settings: Settings,
    services: tuple[str, ...],
    dry_run: bool,
) -> None:
    """Download dataset archives and build the local SQLite index."""
    selected_services = _selected_services(settings, services)
    assets = fetch_dataset_assets(settings.dataset_github_api_url)
    asset = _best_asset_for_services(assets, selected_services)
    if asset is None:
        requested = ", ".join(selected_services)
        raise ConfigError(
            f"No MusicMoveArr dataset torrent found for requested services: {requested}"
        )

    _print_setup_plan(settings, asset, selected_services, dry_run=dry_run)
    if dry_run:
        console.print("No files were downloaded")
        return

    downloader = _require_command(settings.dataset_downloader_command)
    extractor = _require_command(settings.dataset_extractor_command)

    settings.dataset_downloads_dir.mkdir(parents=True, exist_ok=True)
    settings.dataset_staging_dir.mkdir(parents=True, exist_ok=True)

    torrent_path = _download_torrent_file(asset, settings.dataset_downloads_dir)
    selected_indices = _selected_torrent_file_indices(torrent_path, selected_services)
    download_args = [downloader, str(torrent_path), "--dir", str(settings.dataset_downloads_dir)]
    if selected_indices:
        download_args.append(f"--select-file={','.join(str(index) for index in selected_indices)}")
    _run(download_args)
    _extract_archives(
        extractor,
        settings.dataset_downloads_dir,
        settings.dataset_staging_dir,
        selected_services,
    )

    # Detect whether staging contains SQL dumps or CSV files
    sql_files = list(settings.dataset_staging_dir.rglob("*.sql"))
    if sql_files:
        counts = import_raw_tables(
            settings.dataset_staging_dir,
            settings.dataset_index_path,
            selected_services,
        )
        album_rows = sum(counts.values())
        track_rows = 0  # tracks are part of the raw tables
    else:
        album_rows, track_rows = build_index_from_csv_tree(
            settings.dataset_staging_dir,
            settings.dataset_index_path,
            selected_services,
        )
    state = DatasetState(
        version=asset.version,
        services=list(selected_services),
        source_file=asset.name,
        built_at=datetime.now(timezone.utc).isoformat(),
        album_rows=album_rows,
        track_rows=track_rows,
    )
    save_dataset_state(settings.dataset_state_path, state)
    print_success(f"Built local dataset index with {album_rows} album row(s)")


def execute_build(settings: Settings, services: tuple[str, ...]) -> None:
    """Build SQLite index from already-staged dataset files (no download)."""
    selected_services = _selected_services(settings, services)

    if not any(settings.dataset_staging_dir.iterdir()):
        raise ConfigError(
            f"No staged dataset files found in {settings.dataset_staging_dir}. "
            "Run 'auto-tag dataset setup' first or place .sql/.csv files there."
        )

    sql_files = list(settings.dataset_staging_dir.rglob("*.sql"))
    if sql_files:
        counts = import_raw_tables(
            settings.dataset_staging_dir,
            settings.dataset_index_path,
            selected_services,
        )
        album_rows = sum(counts.values())
        track_rows = 0
    else:
        album_rows, track_rows = build_index_from_csv_tree(
            settings.dataset_staging_dir,
            settings.dataset_index_path,
            selected_services,
        )

    state = DatasetState(
        version="manual-build",
        services=list(selected_services),
        source_file="staged data",
        built_at=datetime.now(timezone.utc).isoformat(),
        album_rows=album_rows,
        track_rows=track_rows,
    )
    save_dataset_state(settings.dataset_state_path, state)
    print_success(f"Built local dataset index with {album_rows} total rows imported")


def _selected_services(settings: Settings, services: tuple[str, ...]) -> tuple[str, ...]:
    values = services or tuple(settings.dataset_services)
    selected = tuple(
        dict.fromkeys(service.strip().lower() for service in values if service.strip())
    )
    invalid = sorted(set(selected) - set(SUPPORTED_SERVICES))
    if invalid:
        raise ConfigError(f"Unsupported dataset services: {', '.join(invalid)}")
    return selected or ("musicbrainz",)


def _best_asset_for_services(
    assets: list[DatasetAsset],
    selected_services: tuple[str, ...],
) -> DatasetAsset | None:
    requested = set(selected_services)
    for asset in assets:
        if requested.issubset(set(asset.services)):
            return asset
    return None


def _print_setup_plan(
    settings: Settings,
    asset: DatasetAsset,
    services: tuple[str, ...],
    *,
    dry_run: bool,
) -> None:
    print_info("Dataset setup plan:")
    print_table(
        "Dataset setup",
        ["Key", "Value"],
        [
            ["Mode", "dry run" if dry_run else "download and build"],
            ["Version", asset.version],
            ["Services", ", ".join(services)],
            ["Torrent", asset.name],
            ["Data directory", settings.data_dir],
            ["Downloads", settings.dataset_downloads_dir],
            ["SQLite index", settings.dataset_index_path],
        ],
    )


def _require_command(command: str) -> str:
    resolved = shutil.which(command)
    if resolved is None:
        raise ConfigError(
            f"Required command not found: {command}. "
            "Install aria2c for torrent downloads and 7z for archive extraction."
        )
    return resolved


def _download_torrent_file(asset: DatasetAsset, downloads_dir: Path) -> Path:
    torrent_path = downloads_dir / asset.name
    with urllib.request.urlopen(asset.download_url, timeout=60) as response:
        torrent_path.write_bytes(response.read())
    return torrent_path


def _extract_archives(
    extractor: str,
    downloads_dir: Path,
    staging_dir: Path,
    services: tuple[str, ...],
) -> None:
    archives = [
        archive
        for archive in sorted(downloads_dir.rglob("*.7z"))
        if _archive_matches_services(archive, services)
    ]
    if not archives:
        raise ConfigError(
            f"No .7z archives found in {downloads_dir}. "
            "Check the aria2c download output before running setup again."
        )

    for archive in archives:
        _run([extractor, "x", str(archive), f"-o{staging_dir}", "-y"])


def _run(args: list[str]) -> None:
    try:
        subprocess.run(args, check=True)
    except subprocess.CalledProcessError as exc:
        raise ConfigError(f"Dataset setup command failed: {' '.join(args)}") from exc


def _selected_torrent_file_indices(torrent_path: Path, services: tuple[str, ...]) -> list[int]:
    paths = _torrent_file_paths(torrent_path)
    service_matches = [
        index
        for index, path in enumerate(paths, start=1)
        if path.lower().endswith(".7z") and _archive_matches_services(Path(path), services)
    ]
    if service_matches:
        return service_matches
    return [index for index, path in enumerate(paths, start=1) if path.lower().endswith(".7z")]


def _archive_matches_services(path: Path, services: tuple[str, ...]) -> bool:
    name = str(path).casefold()
    service_names = [service for service in services if service in name]
    return bool(service_names) or not any(service in name for service in SUPPORTED_SERVICES)


def _torrent_file_paths(torrent_path: Path) -> list[str]:
    try:
        decoded, _ = _bdecode(torrent_path.read_bytes())
        info = decoded.get(b"info", {}) if isinstance(decoded, dict) else {}
        files = info.get(b"files")
        if isinstance(files, list):
            return [_torrent_file_path(file_info) for file_info in files]
        name = info.get(b"name", torrent_path.name)
        return [_decode_bencode_text(name)]
    except (OSError, ValueError, TypeError):
        return []


def _torrent_file_path(file_info: Any) -> str:
    if not isinstance(file_info, dict):
        return ""
    parts = file_info.get(b"path", [])
    if not isinstance(parts, list):
        return ""
    return "/".join(_decode_bencode_text(part) for part in parts)


def _decode_bencode_text(value: Any) -> str:
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="replace")
    return str(value)


def _bdecode(data: bytes, index: int = 0) -> tuple[Any, int]:
    token = data[index : index + 1]
    if token == b"i":
        end = data.index(b"e", index)
        return int(data[index + 1 : end]), end + 1
    if token == b"l":
        return _bdecode_list(data, index + 1)
    if token == b"d":
        return _bdecode_dict(data, index + 1)
    if token.isdigit():
        return _bdecode_bytes(data, index)
    raise ValueError("invalid bencode payload")


def _bdecode_list(data: bytes, index: int) -> tuple[list[Any], int]:
    items: list[Any] = []
    while data[index : index + 1] != b"e":
        item, index = _bdecode(data, index)
        items.append(item)
    return items, index + 1


def _bdecode_dict(data: bytes, index: int) -> tuple[dict[bytes, Any], int]:
    items: dict[bytes, Any] = {}
    while data[index : index + 1] != b"e":
        key, index = _bdecode_bytes(data, index)
        value, index = _bdecode(data, index)
        items[key] = value
    return items, index + 1


def _bdecode_bytes(data: bytes, index: int) -> tuple[bytes, int]:
    colon = data.index(b":", index)
    length = int(data[index:colon])
    start = colon + 1
    end = start + length
    return data[start:end], end
