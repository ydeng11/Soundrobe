"""Logging configuration for auto_tagger."""

import logging
import sys
from typing import TextIO

from rich.console import Console
from rich.logging import RichHandler


def setup_logging(
    verbose: bool = False,
    log_file: str | None = None,
    console_output: TextIO | None = None,
) -> logging.Logger:
    """Configure logging with Rich formatting.

    Args:
        verbose: Enable debug-level logging
        log_file: Optional file path for logging
        console_output: Optional console output stream (default: sys.stderr)

    Returns:
        Configured logger instance
    """
    level = logging.DEBUG if verbose else logging.INFO

    handlers: list[logging.Handler] = [
        RichHandler(
            console=Console(file=console_output or sys.stderr),
            rich_tracebacks=True,
            markup=True,
            show_time=False,
            show_path=verbose,
        )
    ]

    if log_file:
        file_handler = logging.FileHandler(log_file)
        file_handler.setLevel(level)
        file_handler.setFormatter(
            logging.Formatter(
                "%(asctime)s - %(name)s - %(levelname)s - %(message)s",
                datefmt="%Y-%m-%d %H:%M:%S",
            )
        )
        handlers.append(file_handler)

    logging.basicConfig(
        level=level,
        format="%(message)s",
        datefmt="[%X]",
        handlers=handlers,
        force=True,
    )

    logger = logging.getLogger("auto_tagger")
    logger.setLevel(level)

    # Suppress noisy dependency loggers at INFO level
    for noisy_logger in ("musicbrainzngs", "httpx"):
        logging.getLogger(noisy_logger).setLevel(logging.WARNING)

    return logger


def get_logger(name: str | None = None) -> logging.Logger:
    """Get a logger instance.

    Args:
        name: Optional logger name (default: auto_tagger)

    Returns:
        Logger instance
    """
    return logging.getLogger(name or "auto_tagger")
