"""Tests for folder-structure fallback lookup."""



def test_parse_album_path_uses_artist_album_parts(tmp_path):
    """Album directories parse as Artist/Album lookup hints."""
    from auto_tagger.integrations.fallback import parse_album_path

    album_path = tmp_path / "Artist" / "Album"
    album_path.mkdir(parents=True)

    request = parse_album_path(album_path)

    assert request.artist_hint == "Artist"
    assert request.album_hint == "Album"
    assert request.path == album_path


def test_parse_album_path_handles_file_paths(tmp_path):
    """File paths use parent as album and grandparent as artist."""
    from auto_tagger.integrations.fallback import parse_album_path

    track_path = tmp_path / "Artist" / "Album" / "01 Song.flac"
    track_path.parent.mkdir(parents=True)
    track_path.write_bytes(b"")

    request = parse_album_path(track_path)

    assert request.artist_hint == "Artist"
    assert request.album_hint == "Album"


def test_candidate_from_folder_uses_file_names_when_tags_missing(tmp_path):
    """Fallback candidates carry folder hints and sorted filename track hints."""
    from auto_tagger.integrations.candidates import LookupSource
    from auto_tagger.integrations.fallback import candidate_from_folder, parse_album_path

    album_path = tmp_path / "Various Artists" / "Soundtrack"
    album_path.mkdir(parents=True)
    (album_path / "02 Second.flac").write_bytes(b"")
    (album_path / "01 First.mp3").write_bytes(b"")

    candidate = candidate_from_folder(parse_album_path(album_path))

    assert candidate.source is LookupSource.FOLDER
    assert candidate.artist == "Various Artists"
    assert candidate.album == "Soundtrack"
    assert candidate.album_artist == "Various Artists"
    # Filename parsing now strips track-number prefixes
    assert [track.title for track in candidate.tracks] == ["First", "Second"]
    assert [track.track_number for track in candidate.tracks] == [1, 2]
    assert candidate.musicbrainz_albumid is None
