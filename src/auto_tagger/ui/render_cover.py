"""Render cover art as coloured terminal blocks for the TUI cover preview.

Uses Unicode half-block characters (▄) where the foreground colour is
the top pixel and the background colour is the bottom pixel, effectively
doubling vertical resolution (2 image pixels per character cell).

Requires Pillow for image decoding and resizing.  When Pillow is not
available, falls back to a plain-text description.
"""

from __future__ import annotations

from pathlib import Path

from rich.style import Style
from rich.text import Text

# ── Public API ─────────────────────────────────────────────────────────────────


def render_cover_from_bytes(
    data: bytes,
    max_width: int = 30,
    max_height: int = 10,
) -> Text | None:
    """Convert raw image bytes into a coloured Rich ``Text`` renderable.

    Returns ``None`` when Pillow is not installed or the image cannot be
    decoded.
    """
    try:
        from PIL import Image as PILImage

        import io

        img: PILImage.Image = PILImage.open(io.BytesIO(data))
        return _render_pil_image(img, max_width, max_height)
    except Exception:
        return None


def render_cover_from_path(
    path: Path,
    max_width: int = 30,
    max_height: int = 10,
) -> Text | None:
    """Load a cover image from a file and return a coloured renderable.

    Returns ``None`` when Pillow is not installed or the file cannot be
    decoded.
    """
    try:
        from PIL import Image as PILImage

        img: PILImage.Image = PILImage.open(str(path))
        return _render_pil_image(img, max_width, max_height)
    except Exception:
        return None


# ── Internal ───────────────────────────────────────────────────────────────────


def _render_pil_image(
    img: object,
    max_width: int,
    max_height: int,
) -> Text | None:
    """Render a Pillow Image as a coloured half-block grid.

    The image is resized (preserving aspect ratio) so that its width fits
    *max_width* and its height fits *max_height* character rows.  Because
    each half-block character (``▄``) displays **two** vertical image pixels,
    the image height after resizing is ``rows * 2`` pixels.

    Returns a ``rich.text.Text`` object with one ``▄`` per cell.
    """
    from PIL import Image as PILImage

    pil_img: PILImage.Image = img  # type: ignore[assignment]

    # Convert to RGB (handles RGBA -> discard alpha, palette -> RGB)
    if pil_img.mode not in ("RGB", "RGBA"):
        pil_img = pil_img.convert("RGB")  # type: ignore[union-attr]
    elif pil_img.mode == "RGBA":
        # Composite onto black background for predictable results
        bg = PILImage.new("RGB", pil_img.size, (0, 0, 0))
        bg.paste(pil_img, mask=pil_img.split()[3])
        pil_img = bg

    w_px, h_px = pil_img.size

    # Maximum pixel dimensions given the character constraints
    # (each character row shows 2 image rows when using half-blocks).
    max_px_w = max_width
    max_px_h = max_height * 2

    # Scale to fit, preserving aspect ratio
    scale = min(max_px_w / w_px, max_px_h / h_px, 1.0)
    new_w = max(1, round(w_px * scale))
    new_h = max(2, round(h_px * scale))

    import warnings

    resized = pil_img.resize((new_w, new_h), PILImage.LANCZOS)
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", DeprecationWarning)
        pixels = list(resized.getdata())
    rows = new_h // 2  # number of half-block character rows

    result = Text()
    stride = new_w

    for row in range(rows):
        for col in range(new_w):
            idx_top = row * 2 * stride + col
            idx_bot = (row * 2 + 1) * stride + col

            top_rgb = pixels[idx_top] if idx_top < len(pixels) else (0, 0, 0)
            bot_rgb = pixels[idx_bot] if idx_bot < len(pixels) else (0, 0, 0)

            style = Style(
                color=f"rgb({top_rgb[0]},{top_rgb[1]},{top_rgb[2]})",
                bgcolor=f"rgb({bot_rgb[0]},{bot_rgb[1]},{bot_rgb[2]})",
            )
            result.append("▄", style=style)

        newline_style = Style()
        result.append("\n", style=newline_style)

    return result
