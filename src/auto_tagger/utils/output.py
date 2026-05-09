"""Output formatting utilities."""

from typing import Any

from rich.console import Console
from rich.table import Table

console = Console()


def print_success(message: str) -> None:
    """Print success message in green."""
    console.print(f"[green]✓[/green] {message}")


def print_error(message: str) -> None:
    """Print error message in red."""
    console.print(f"[red]✗[/red] {message}", style="red")


def print_warning(message: str) -> None:
    """Print warning message in yellow."""
    console.print(f"[yellow]![/yellow] {message}", style="yellow")


def print_info(message: str) -> None:
    """Print info message in blue."""
    console.print(f"[blue]ℹ[/blue] {message}")


def print_table(
    title: str,
    columns: list[str],
    rows: list[list[Any]],
    show_header: bool = True,
) -> None:
    """Print data as a table.

    Args:
        title: Table title
        columns: Column headers
        rows: Table rows (list of values)
        show_header: Whether to show column headers
    """
    table = Table(title=title, show_header=show_header)

    for column in columns:
        table.add_column(column)

    for row in rows:
        table.add_row(*[str(cell) for cell in row])

    console.print(table)


def print_json(data: Any) -> None:
    """Print data as formatted JSON."""
    import json

    from rich.syntax import Syntax

    json_str = json.dumps(data, indent=2, sort_keys=True)
    syntax = Syntax(json_str, "json", theme="monokai", line_numbers=False)
    console.print(syntax)


def print_panel(content: str, title: str | None = None, style: str = "blue") -> None:
    """Print content in a panel.

    Args:
        content: Panel content
        title: Optional panel title
        style: Panel border style
    """
    from rich.panel import Panel

    panel = Panel(content, title=title, border_style=style)
    console.print(panel)
