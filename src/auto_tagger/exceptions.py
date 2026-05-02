"""Custom exceptions for auto_tagger."""


class AutoTaggerError(Exception):
    """Base exception for auto_tagger."""
    exit_code = 1


class ConfigError(AutoTaggerError):
    """Configuration related errors."""
    exit_code = 2


class FileProcessingError(AutoTaggerError):
    """File processing errors."""
    exit_code = 3


class ValidationError(AutoTaggerError):
    """Validation errors."""
    exit_code = 4


class TaggingError(AutoTaggerError):
    """Tagging operation errors."""
    exit_code = 5