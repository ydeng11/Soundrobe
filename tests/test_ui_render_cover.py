"""Tests for the cover art terminal rendering module."""

from __future__ import annotations

import io
from pathlib import Path
from tempfile import NamedTemporaryFile

import pytest

from auto_tagger.ui.render_cover import render_cover_from_bytes, render_cover_from_path


class TestRenderCoverFromBytes:
    """Tests for render_cover_from_bytes."""

    def test_valid_jpeg_returns_text(self):
        """A valid JPEG produces a Rich Text renderable."""
        from PIL import Image as PILImage

        img = PILImage.new("RGB", (40, 40))
        pixels = img.load()
        for y in range(40):
            for x in range(40):
                pixels[x, y] = (255, 0, 0)  # solid red
        buf = io.BytesIO()
        img.save(buf, format="JPEG")
        data = buf.getvalue()

        result = render_cover_from_bytes(data, max_width=20, max_height=8)
        assert result is not None
        assert result.plain
        # Should contain half-block characters
        assert "▄" in result.plain
        # Should have newlines (one per row)
        assert "\n" in result.plain

    def test_valid_png_returns_text(self):
        """A valid PNG produces a Rich Text renderable."""
        from PIL import Image as PILImage

        img = PILImage.new("RGBA", (30, 30), (0, 128, 0, 255))
        buf = io.BytesIO()
        img.save(buf, format="PNG")
        data = buf.getvalue()

        result = render_cover_from_bytes(data, max_width=15, max_height=6)
        assert result is not None
        assert "▄" in result.plain

    def test_invalid_data_returns_none(self):
        """Bogus image data returns None."""
        result = render_cover_from_bytes(b"not an image")
        assert result is None

    def test_empty_data_returns_none(self):
        """Empty bytes returns None."""
        result = render_cover_from_bytes(b"")
        assert result is None

    def test_dimensions_are_respected(self):
        """Rendered output fits within max_width x max_height."""
        from PIL import Image as PILImage

        img = PILImage.new("RGB", (200, 200))
        buf = io.BytesIO()
        img.save(buf, format="JPEG")
        data = buf.getvalue()

        result = render_cover_from_bytes(data, max_width=10, max_height=4)
        assert result is not None
        lines = result.plain.split("\n")
        # Should have at most max_height lines (minus trailing empty line)
        non_empty = [l for l in lines if l.strip()]
        assert len(non_empty) <= 4
        # Each line should be at most max_width chars
        for line in non_empty:
            assert len(line) <= 10

    def test_grayscale_image_works(self):
        """Images with mode 'L' (grayscale) convert successfully."""
        from PIL import Image as PILImage

        img = PILImage.new("L", (50, 50))
        pixels = img.load()
        for y in range(50):
            for x in range(50):
                pixels[x, y] = x * 5  # gradient
        buf = io.BytesIO()
        img.save(buf, format="PNG")
        data = buf.getvalue()

        result = render_cover_from_bytes(data, max_width=20, max_height=8)
        assert result is not None
        assert "▄" in result.plain


class TestRenderCoverFromPath:
    """Tests for render_cover_from_path."""

    def test_valid_path_returns_text(self):
        """A valid image file path produces a Rich Text renderable."""
        from PIL import Image as PILImage

        img = PILImage.new("RGB", (50, 50), (0, 0, 255))
        with NamedTemporaryFile(suffix=".png", delete=False) as f:
            img.save(f, format="PNG")
            tmp_path = Path(f.name)

        try:
            result = render_cover_from_path(tmp_path, max_width=20, max_height=8)
            assert result is not None
            assert "▄" in result.plain
        finally:
            tmp_path.unlink(missing_ok=True)

    def test_missing_path_returns_none(self):
        """A non-existent file path returns None."""
        result = render_cover_from_path(Path("/nonexistent/cover.jpg"))
        assert result is None

    def test_invalid_file_returns_none(self):
        """A file that exists but is not an image returns None."""
        with NamedTemporaryFile(suffix=".txt", delete=False) as f:
            f.write(b"this is not an image")
            tmp_path = Path(f.name)

        try:
            result = render_cover_from_path(tmp_path)
            assert result is None
        finally:
            tmp_path.unlink(missing_ok=True)
