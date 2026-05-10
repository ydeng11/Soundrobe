"""Tests for dataset setup and status commands."""

from click.testing import CliRunner

from auto_tagger.cli import cli


def test_dataset_status_reports_missing_index(tmp_path):
    """Dataset status shows the configured dot-folder paths before setup."""
    runner = CliRunner()

    result = runner.invoke(
        cli,
        ["dataset", "status"],
        env={"AUTO_TAG_DATA_DIR": str(tmp_path)},
    )

    assert result.exit_code == 0
    assert "not installed" in result.output.lower()
    assert str(tmp_path / "dataset-index.sqlite") in result.output


def test_dataset_setup_dry_run_prints_latest_plan(monkeypatch, tmp_path):
    """Setup dry-run reads repository metadata without downloading archives."""
    from auto_tagger.integrations.dataset import DatasetAsset

    monkeypatch.setattr(
        "auto_tagger.commands.dataset.fetch_dataset_assets",
        lambda *_args, **_kwargs: [
            DatasetAsset(
                version="22 Feb 2026",
                name="MusicBrainz Tidal Spotify Deezer Dataset 22 Feb 2026.torrent",
                download_url="https://example.invalid/dataset.torrent",
                services=["musicbrainz", "tidal", "spotify", "deezer"],
            )
        ],
    )

    runner = CliRunner()
    result = runner.invoke(
        cli,
        ["dataset", "setup", "--dry-run"],
        env={"AUTO_TAG_DATA_DIR": str(tmp_path)},
    )

    assert result.exit_code == 0
    assert "22 Feb 2026" in result.output
    assert "musicbrainz" in result.output
    assert "No files were downloaded" in result.output


def test_dataset_setup_selects_matching_torrent_archives(tmp_path):
    """Setup narrows aria2c torrent file selection to requested service archives."""
    from auto_tagger.commands.dataset import _selected_torrent_file_indices

    torrent_path = tmp_path / "dataset.torrent"
    torrent_path.write_bytes(
        _bencode(
            {
                b"info": {
                    b"files": [
                        {b"path": [b"musicbrainz_22_feb_2026.7z"]},
                        {b"path": [b"spotify_22_feb_2026.7z"]},
                        {b"path": [b"tidal_22_feb_2026.7z"]},
                    ]
                }
            }
        )
    )

    assert _selected_torrent_file_indices(torrent_path, ("musicbrainz",)) == [1]


def _bencode(value):
    if isinstance(value, dict):
        body = b"".join(_bencode(key) + _bencode(value[key]) for key in sorted(value))
        return b"d" + body + b"e"
    if isinstance(value, list):
        return b"l" + b"".join(_bencode(item) for item in value) + b"e"
    if isinstance(value, bytes):
        return str(len(value)).encode() + b":" + value
    if isinstance(value, int):
        return b"i" + str(value).encode() + b"e"
    raise TypeError(value)
