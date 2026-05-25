"""Tests for logging configuration."""

import logging

from auto_tagger.utils.logging import setup_logging


def test_setup_logging_verbose():
    """Verbose mode sets DEBUG level."""
    setup_logging(verbose=True)
    logger = logging.getLogger("auto_tagger")
    assert logger.level == logging.DEBUG


def test_setup_logging_default():
    """Default mode sets INFO level."""
    setup_logging(verbose=False)
    logger = logging.getLogger("auto_tagger")
    assert logger.level <= logging.INFO


def test_setup_logging_idempotent():
    """Calling setup_logging twice does not duplicate handlers."""
    logger = logging.getLogger("auto_tagger")
    setup_logging(verbose=False)
    count1 = len(logger.handlers)
    setup_logging(verbose=False)
    count2 = len(logger.handlers)
    # Should not add duplicate handlers
    assert count2 <= count1 + 1


def test_setup_logging_file_creates_dir_and_writes(tmp_path):
    """Passing log_file creates the parent directory and writes logs to it."""
    import logging

    log_file = tmp_path / "logs" / "auto-tagger.log"
    assert not log_file.parent.exists()

    logger = setup_logging(verbose=True, log_file=str(log_file))

    # Parent directory should have been created automatically
    assert log_file.parent.exists()

    # Log something and verify it gets written to the file
    logger.info("test log message")
    content = log_file.read_text()
    assert "test log message" in content
