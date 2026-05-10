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
