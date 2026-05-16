"""CLI command for enriching artist aliases from MusicBrainz."""

from __future__ import annotations

import musicbrainzngs as mb  # type: ignore[import-untyped]

from auto_tagger.config import Settings
from auto_tagger.integrations.aliases import (
    ALIAS_FILE,
    get_aliases,
    is_chinese_name,
    load_aliases,
    save_alias,
)
from auto_tagger.utils import console

_mb_ua_initialized = False


def _ensure_mb_useragent() -> None:
    global _mb_ua_initialized
    if not _mb_ua_initialized:
        try:
            mb.set_useragent(
                "auto-tagger", "0.1.0",
                "https://github.com/auto-tagger/auto-tagger",
            )
        except Exception:
            pass
        _mb_ua_initialized = True


def execute_enrich(settings: Settings, dry_run: bool) -> None:
    """Enrich existing Chinese artist aliases with English/Pinyin/TC names.

    Only processes entries in the alias file whose key name contains Chinese
    characters AND have no Latin-script alias yet. Queries MusicBrainz for
    each such artist and seeds the discovered aliases.
    """
    if not ALIAS_FILE.exists():
        console.print("[yellow]No alias file found at:[/yellow]", str(ALIAS_FILE))
        console.print("Nothing to enrich.")
        return

    aliases = load_aliases()
    if not aliases:
        console.print("[yellow]Alias file is empty. Nothing to enrich.")
        return

    # Find Chinese-named entries with no Latin-script alias
    to_enrich: list[str] = []
    for key in aliases:
        existing = aliases[key]
        has_latin = any(a.isascii() for a in existing)
        if is_chinese_name(key) and not has_latin:
            to_enrich.append(key)

    if not to_enrich:
        console.print("[green]All Chinese artist entries already have Latin-script aliases.")
        return

    console.print(f"Found [cyan]{len(to_enrich)}[/cyan] Chinese artist(s) to enrich:")
    for key in to_enrich:
        console.print(f"  [dim]{key}[/dim] (existing aliases: {aliases[key]})")

    if dry_run:
        console.print("\n[dim]Dry run — no changes saved.[/dim]")
        return

    _ensure_mb_useragent()

    enriched = 0
    failed = 0
    with console.status("Querying MusicBrainz...") as status:
        for key in to_enrich:
            status.update(f"Querying [bold]{key}[/bold]...")
            try:
                result = mb.search_artists(artist=key, limit=1)
                artist_list = result.get("artist-list") or []
                if not artist_list:
                    console.print(f"  [red]✗[/red] {key}: not found on MusicBrainz")
                    failed += 1
                    continue

                alias_list = artist_list[0].get("alias-list") or []
                if not alias_list:
                    console.print(f"  [yellow]~[/yellow] {key}: no aliases on MusicBrainz")
                    failed += 1
                    continue

                count = 0
                for alias_entry in alias_list:
                    alias_text = alias_entry.get("alias") or alias_entry.get("name")
                    if alias_text and alias_text.strip():
                        save_alias(key, alias_text.strip())
                        count += 1
                console.print(f"  [green]✓[/green] {key}: seeded {count} alias(es)")
                enriched += 1
            except Exception as exc:
                console.print(f"  [red]✗[/red] {key}: {exc}")
                failed += 1

    console.print()
    console.print(
        f"[green]Done:[/green] {enriched} enriched, {failed} failed."
    )
