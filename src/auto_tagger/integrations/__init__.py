"""External metadata lookup integrations."""

from auto_tagger.integrations.beets_client import BeetsClient
from auto_tagger.integrations.cache import MatchCache
from auto_tagger.integrations.candidates import (
    AlbumCandidate,
    LookupRequest,
    LookupSource,
    TrackCandidate,
)
from auto_tagger.integrations.dataset import DatasetAsset, DatasetIndexClient, DatasetIndexWriter
from auto_tagger.integrations.lookup import LookupService

__all__ = [
    "AlbumCandidate",
    "BeetsClient",
    "DatasetAsset",
    "DatasetIndexClient",
    "DatasetIndexWriter",
    "LookupRequest",
    "LookupService",
    "LookupSource",
    "MatchCache",
    "TrackCandidate",
]
