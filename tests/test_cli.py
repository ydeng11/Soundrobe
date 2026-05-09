"""Tests for CLI functionality."""

from click.testing import CliRunner

from auto_tagger.cli import cli


def test_cli_help():
    """Test CLI help output."""
    runner = CliRunner()
    result = runner.invoke(cli, ["--help"])
    assert result.exit_code == 0
    assert "Auto Tagger" in result.output
    assert "tag" in result.output
    assert "batch" in result.output


def test_cli_version():
    """Test version output."""
    runner = CliRunner()
    result = runner.invoke(cli, ["--version"])
    assert result.exit_code == 0
    assert "0.1.0" in result.output


def test_tag_command_help():
    """Test tag command help."""
    runner = CliRunner()
    result = runner.invoke(cli, ["tag", "--help"])
    assert result.exit_code == 0
    assert "Tag a single album" in result.output


def test_batch_command_help():
    """Test batch command help."""
    runner = CliRunner()
    result = runner.invoke(cli, ["batch", "--help"])
    assert result.exit_code == 0
    assert "Batch process" in result.output


def test_config_command():
    """Test config command."""
    runner = CliRunner()
    result = runner.invoke(cli, ["config"])
    assert result.exit_code == 0
    assert "Configuration" in result.output


def test_config_show_key():
    """Test showing config key."""
    runner = CliRunner()
    result = runner.invoke(cli, ["config", "verbose"])
    assert result.exit_code == 0
    assert "verbose" in result.output


def test_config_invalid_key():
    """Test showing invalid config key."""
    runner = CliRunner()
    result = runner.invoke(cli, ["config", "invalid_key"])
    assert result.exit_code == 0
    assert "Unknown configuration key" in result.output


def test_tag_command_with_path(tmp_path):
    """Test tag command with a valid path."""
    runner = CliRunner()
    result = runner.invoke(cli, ["tag", str(tmp_path)])
    assert result.exit_code == 0
    assert "Tagging:" in result.output


def test_tag_command_dry_run(tmp_path):
    """Test tag command with dry-run flag."""
    runner = CliRunner()
    result = runner.invoke(cli, ["tag", str(tmp_path), "--dry-run"])
    assert result.exit_code == 0
    assert "No supported audio files" in result.output


def test_tag_command_dry_run_previews_audio_metadata(tmp_path, monkeypatch):
    """Dry-run tag command previews discovered audio metadata."""
    from auto_tagger.core.metadata import TrackMetadata
    from auto_tagger.integrations.candidates import AlbumCandidate, LookupSource

    audio_file = tmp_path / "01.flac"
    audio_file.write_bytes(b"")

    monkeypatch.setattr(
        "auto_tagger.commands.tag.iter_audio_files",
        lambda path, recursive=False: [audio_file],
    )
    monkeypatch.setattr(
        "auto_tagger.commands.tag.read_metadata",
        lambda path: TrackMetadata(title="Song", artist="Artist", album="Album"),
    )
    monkeypatch.setattr(
        "auto_tagger.commands.tag.LookupService",
        lambda **kwargs: type(
            "FakeLookupService",
            (),
            {
                "lookup_album": lambda self, path: [
                    AlbumCandidate(
                        artist="Artist",
                        album="Album",
                        year="2024",
                        musicbrainz_albumid="album-id",
                        source=LookupSource.BEETS,
                        distance=0.2,
                    )
                ]
            },
        )(),
    )

    runner = CliRunner()
    result = runner.invoke(cli, ["tag", str(tmp_path), "--dry-run"])

    assert result.exit_code == 0
    assert "Metadata preview" in result.output
    assert "Lookup candidates" in result.output
    assert "Song" in result.output
    assert "album-id" in result.output


def test_tag_command_dry_run_notes_llm_unavailable_without_key(tmp_path, monkeypatch):
    """Dry-run lookup preview does not call LLM without an API key."""
    from auto_tagger.core.metadata import TrackMetadata
    from auto_tagger.integrations.candidates import AlbumCandidate, LookupSource

    audio_file = tmp_path / "01.flac"
    audio_file.write_bytes(b"")

    monkeypatch.setattr(
        "auto_tagger.commands.tag.iter_audio_files",
        lambda path, recursive=False: [audio_file],
    )
    monkeypatch.setattr(
        "auto_tagger.commands.tag.read_metadata",
        lambda path: TrackMetadata(title="Song", artist="Artist", album="Album"),
    )
    monkeypatch.setattr(
        "auto_tagger.commands.tag.LookupService",
        lambda **kwargs: type(
            "FakeLookupService",
            (),
            {
                "lookup_album": lambda self, path: [
                    AlbumCandidate(artist="Artist", album="A", source=LookupSource.BEETS),
                    AlbumCandidate(artist="Artist", album="B", source=LookupSource.BEETS),
                ]
            },
        )(),
    )

    runner = CliRunner()
    result = runner.invoke(cli, ["tag", str(tmp_path), "--dry-run"])

    assert result.exit_code == 0
    assert "LLM selection unavailable" in result.output


def test_tag_command_dry_run_shows_llm_selection_with_api_key(tmp_path, monkeypatch):
    """Dry-run lookup preview displays LLM selection when an API key is configured."""
    from auto_tagger.core.metadata import TrackMetadata
    from auto_tagger.integrations.candidates import AlbumCandidate, LookupRequest, LookupSource
    from auto_tagger.llm.selection import SelectionResult

    audio_file = tmp_path / "01.flac"
    audio_file.write_bytes(b"")
    candidates = [
        AlbumCandidate(artist="Artist", album="A", source=LookupSource.BEETS),
        AlbumCandidate(artist="Artist", album="B", source=LookupSource.BEETS),
    ]

    class FakeLookupService:
        def lookup_album(self, path):
            return candidates

        def request_from_path(self, path):
            return LookupRequest(path=path, artist_hint="Artist", album_hint="Album")

    class FakeSelectionService:
        def __init__(self, client, settings):
            pass

        def select_candidate(self, request, lookup_candidates):
            return SelectionResult(candidates[1], 0.91, "best match")

    monkeypatch.setattr(
        "auto_tagger.commands.tag.iter_audio_files",
        lambda path, recursive=False: [audio_file],
    )
    monkeypatch.setattr(
        "auto_tagger.commands.tag.read_metadata",
        lambda path: TrackMetadata(title="Song", artist="Artist", album="Album"),
    )
    monkeypatch.setattr(
        "auto_tagger.commands.tag.LookupService",
        lambda **kwargs: FakeLookupService(),
    )
    monkeypatch.setattr("auto_tagger.commands.tag.OpenRouterClient", lambda settings: object())
    monkeypatch.setattr("auto_tagger.commands.tag.CandidateSelectionService", FakeSelectionService)

    runner = CliRunner()
    result = runner.invoke(
        cli,
        ["tag", str(tmp_path), "--dry-run"],
        env={"AUTO_TAG_LLM_API_KEY": "key"},
    )

    assert result.exit_code == 0
    assert "LLM selection" in result.output
    assert "0.91" in result.output
    assert "B" in result.output


def test_verbose_flag(tmp_path):
    """Test verbose flag."""
    runner = CliRunner()
    result = runner.invoke(cli, ["--verbose", "tag", str(tmp_path)])
    assert result.exit_code == 0


def test_output_format(tmp_path):
    """Test output format option."""
    runner = CliRunner()
    result = runner.invoke(cli, ["--output", "json", "config"])
    assert result.exit_code == 0


def test_tag_nonexistent_path():
    """Test tag command with nonexistent path."""
    runner = CliRunner()
    result = runner.invoke(cli, ["tag", "/nonexistent/path"])
    assert result.exit_code != 0
