use std::future::Future;
use std::pin::Pin;

use super::providers::{
    DiscogsClient, MusicBrainzClient, ProviderAlbum, ReleaseSearchSummary, ReleaseSummaryPage,
};

pub type TagSourceFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, TagSourceError>> + Send + 'a>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagSourceError {
    Unavailable(String),
    Incomplete(String),
    Cancelled,
}

impl std::fmt::Display for TagSourceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(message) | Self::Incomplete(message) => formatter.write_str(message),
            Self::Cancelled => formatter.write_str("provider request cancelled"),
        }
    }
}

impl std::error::Error for TagSourceError {}

pub fn classify_provider_error(message: impl Into<String>) -> TagSourceError {
    let message = message.into();
    if message.to_ascii_lowercase().contains("cancel") {
        TagSourceError::Cancelled
    } else if message.to_ascii_lowercase().contains("incomplete")
        || message.to_ascii_lowercase().contains("budget")
    {
        TagSourceError::Incomplete(message)
    } else {
        TagSourceError::Unavailable(message)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagSourcePage {
    pub summaries: Vec<ReleaseSearchSummary>,
    pub total: Option<u32>,
    pub returned: u32,
    pub next_cursor: Option<u32>,
}

pub trait TagSource: Send + Sync {
    fn name(&self) -> &'static str;

    fn resolve_artist_id<'a>(&'a self, artist: &'a str) -> TagSourceFuture<'a, Option<String>>;

    fn release_page<'a>(
        &'a self,
        artist_id: &'a str,
        cursor: u32,
        limit: u32,
    ) -> TagSourceFuture<'a, TagSourcePage>;

    fn release_detail<'a>(&'a self, release_id: &'a str) -> TagSourceFuture<'a, ProviderAlbum>;
}

fn musicbrainz_page(page: ReleaseSummaryPage) -> TagSourcePage {
    let next_cursor = page.next_offset();
    TagSourcePage {
        summaries: page.summaries,
        total: page.total,
        returned: page.returned,
        next_cursor,
    }
}

fn discogs_page(page: ReleaseSummaryPage) -> TagSourcePage {
    let next_cursor = page.next_page();
    TagSourcePage {
        summaries: page.summaries,
        total: page.total,
        returned: page.returned,
        next_cursor,
    }
}

impl TagSource for MusicBrainzClient {
    fn name(&self) -> &'static str {
        "musicbrainz"
    }

    fn resolve_artist_id<'a>(&'a self, artist: &'a str) -> TagSourceFuture<'a, Option<String>> {
        Box::pin(async move {
            self.search_artist_by_name_result(artist)
                .await
                .map(|artist| artist.map(|artist| artist.id))
                .map_err(classify_provider_error)
        })
    }

    fn release_page<'a>(
        &'a self,
        artist_id: &'a str,
        cursor: u32,
        limit: u32,
    ) -> TagSourceFuture<'a, TagSourcePage> {
        Box::pin(async move {
            self.artist_release_summary_page_result(artist_id, cursor, limit)
                .await
                .map(musicbrainz_page)
                .map_err(classify_provider_error)
        })
    }

    fn release_detail<'a>(&'a self, release_id: &'a str) -> TagSourceFuture<'a, ProviderAlbum> {
        Box::pin(async move {
            self.release_by_id_result(release_id)
                .await
                .map_err(classify_provider_error)
        })
    }
}

impl TagSource for DiscogsClient {
    fn name(&self) -> &'static str {
        "discogs"
    }

    fn resolve_artist_id<'a>(&'a self, artist: &'a str) -> TagSourceFuture<'a, Option<String>> {
        Box::pin(async move {
            self.search_artist_exact_result(artist)
                .await
                .map_err(classify_provider_error)
        })
    }

    fn release_page<'a>(
        &'a self,
        artist_id: &'a str,
        cursor: u32,
        limit: u32,
    ) -> TagSourceFuture<'a, TagSourcePage> {
        Box::pin(async move {
            self.artist_release_summary_page_result(artist_id, cursor, limit)
                .await
                .map(discogs_page)
                .map_err(classify_provider_error)
        })
    }

    fn release_detail<'a>(&'a self, release_id: &'a str) -> TagSourceFuture<'a, ProviderAlbum> {
        Box::pin(async move {
            self.release_metadata_result(release_id)
                .await
                .map_err(classify_provider_error)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_errors_keep_incomplete_distinct_from_unavailable() {
        assert_eq!(
            classify_provider_error("Discogs discovery incomplete: request budget reached"),
            TagSourceError::Incomplete(
                "Discogs discovery incomplete: request budget reached".into()
            )
        );
        assert_eq!(
            classify_provider_error("MusicBrainz request failed: 503"),
            TagSourceError::Unavailable("MusicBrainz request failed: 503".into())
        );
    }
}
