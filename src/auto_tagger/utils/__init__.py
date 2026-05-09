"""Utility functions for auto_tagger."""

from auto_tagger.utils.logging import get_logger, setup_logging
from auto_tagger.utils.output import (
    console,
    print_error,
    print_info,
    print_json,
    print_panel,
    print_success,
    print_table,
    print_warning,
)

__all__ = [
    "console",
    "get_logger",
    "print_error",
    "print_info",
    "print_json",
    "print_panel",
    "print_success",
    "print_table",
    "print_warning",
    "setup_logging",
]
