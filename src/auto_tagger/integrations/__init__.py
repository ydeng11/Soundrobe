"""External metadata lookup integrations."""

from auto_tagger.integrations.beets_client import BeetsClient
from auto_tagger.integrations.cache import MatchCache
from auto_tagger.integrations.candidates import (
    AlbumCandidate,
    LookupRequest,
    LookupSource,
    TrackCandidate,
    verify_album_name,
)
from auto_tagger.integrations.dataset import DatasetAsset, DatasetIndexClient, DatasetIndexWriter
from auto_tagger.integrations.discogs_client import DiscogsClient, DiscogsError
from auto_tagger.integrations.lookup import LookupService

__all__ = [
    "AlbumCandidate",
    "BeetsClient",
    "DatasetAsset",
    "DatasetIndexClient",
    "DatasetIndexWriter",
    "DiscogsClient",
    "DiscogsError",
    "LookupRequest",
    "LookupService",
    "LookupSource",
    "MatchCache",
    "TrackCandidate",
    "verify_album_name",
]
