//! Canonical tag names accepted by tagging cleanup settings.
//!
//! The catalog is intentionally bounded to fields that have an ID3 frame or a
//! Navidrome mapping used by Soundrobe's writers. Renderer input is normalized
//! to these IDs before it reaches the tagging pipeline.

use serde::Serialize;

#[derive(Clone, Copy)]
struct TagDefinition {
    id: &'static str,
    label: &'static str,
    frame: &'static str,
    aliases: &'static [&'static str],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagSuggestion {
    pub id: String,
    pub label: String,
    pub frame: String,
    pub source: &'static str,
}

const TAG_SOURCE: &str = "ID3 / Navidrome";

const TAGS: &[TagDefinition] = &[
    TagDefinition {
        id: "title",
        label: "Title",
        frame: "TIT2",
        aliases: &["title", "tit2"],
    },
    TagDefinition {
        id: "artist",
        label: "Artist",
        frame: "TPE1",
        aliases: &["artist", "tpe1"],
    },
    TagDefinition {
        id: "artists",
        label: "Artists",
        frame: "TXXX:ARTISTS",
        aliases: &["artists", "txxx:artists"],
    },
    TagDefinition {
        id: "album",
        label: "Album",
        frame: "TALB",
        aliases: &["album", "talb"],
    },
    TagDefinition {
        id: "albumArtist",
        label: "Album artist",
        frame: "TPE2",
        aliases: &["albumartist", "album artist", "album_artist", "tpe2"],
    },
    TagDefinition {
        id: "albumArtists",
        label: "Album artists",
        frame: "TXXX:ALBUMARTISTS",
        aliases: &["albumartists", "txxx:albumartists"],
    },
    TagDefinition {
        id: "year",
        label: "Year / recording date",
        frame: "TDRC / TYER",
        aliases: &["year", "date", "tdrc", "tyer"],
    },
    TagDefinition {
        id: "genre",
        label: "Genre",
        frame: "TCON",
        aliases: &["genre", "tcon"],
    },
    TagDefinition {
        id: "composer",
        label: "Composer",
        frame: "TCOM",
        aliases: &["composer", "tcom"],
    },
    TagDefinition {
        id: "comment",
        label: "Comment",
        frame: "COMM",
        aliases: &["comment", "comm"],
    },
    TagDefinition {
        id: "description",
        label: "Description",
        frame: "TXXX:DESCRIPTION / COMM:description",
        aliases: &["description", "txxx:description", "comm:description"],
    },
    TagDefinition {
        id: "lyrics",
        label: "Lyrics",
        frame: "USLT",
        aliases: &["lyrics", "uslt", "unsyncedlyrics"],
    },
    TagDefinition {
        id: "compilation",
        label: "Compilation",
        frame: "TCMP",
        aliases: &["compilation", "tcmp", "cpil"],
    },
    TagDefinition {
        id: "trackNumber",
        label: "Track number",
        frame: "TRCK",
        aliases: &["track", "tracknumber", "track number", "trck"],
    },
    TagDefinition {
        id: "trackTotal",
        label: "Track total",
        frame: "TRCK",
        aliases: &["tracktotal", "totaltracks"],
    },
    TagDefinition {
        id: "discNumber",
        label: "Disc number",
        frame: "TPOS",
        aliases: &["disc", "discnumber", "disc number", "tpos"],
    },
    TagDefinition {
        id: "discTotal",
        label: "Disc total",
        frame: "TPOS",
        aliases: &["disctotal", "totaldiscs"],
    },
    TagDefinition {
        id: "musicbrainzTrackId",
        label: "MusicBrainz track ID",
        frame: "TXXX:MusicBrainz Track Id",
        aliases: &[
            "musicbrainztrackid",
            "musicbrainz track id",
            "musicbrainz_trackid",
        ],
    },
    TagDefinition {
        id: "musicbrainzAlbumId",
        label: "MusicBrainz album ID",
        frame: "TXXX:MusicBrainz Album Id",
        aliases: &[
            "musicbrainzalbumid",
            "musicbrainz album id",
            "musicbrainz_albumid",
        ],
    },
    TagDefinition {
        id: "musicbrainzArtistId",
        label: "MusicBrainz artist ID",
        frame: "TXXX:MusicBrainz Artist Id",
        aliases: &[
            "musicbrainzartistid",
            "musicbrainz artist id",
            "musicbrainz_artistid",
        ],
    },
    TagDefinition {
        id: "discogsArtistId",
        label: "Discogs artist ID",
        frame: "TXXX:Discogs Artist Id",
        aliases: &["discogsartistid", "discogs artist id", "discogs_artist_id"],
    },
    TagDefinition {
        id: "discogsReleaseId",
        label: "Discogs release ID",
        frame: "TXXX:Discogs Release Id",
        aliases: &[
            "discogsreleaseid",
            "discogs release id",
            "discogs_release_id",
        ],
    },
];

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn definition_for(value: &str) -> Option<&'static TagDefinition> {
    let normalized = normalize(value);
    TAGS.iter().find(|definition| {
        normalize(definition.id) == normalized
            || definition
                .aliases
                .iter()
                .any(|alias| normalize(alias) == normalized)
    })
}

pub fn canonical_tag_id(value: &str) -> Option<&'static str> {
    definition_for(value).map(|definition| definition.id)
}

pub fn canonicalize_clear_tags(values: &[String]) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some(id) = canonical_tag_id(trimmed) else {
            return Err(format!("unsupported tag \"{trimmed}\""));
        };
        if !result.iter().any(|existing| existing == id) {
            result.push(id.to_string());
        }
    }
    Ok(result)
}

pub fn suggestions(query: &str) -> Vec<TagSuggestion> {
    let normalized_query = normalize(query);
    TAGS.iter()
        .filter(|definition| {
            normalized_query.is_empty()
                || normalize(definition.id).contains(&normalized_query)
                || normalize(definition.label).contains(&normalized_query)
                || definition
                    .aliases
                    .iter()
                    .any(|alias| normalize(alias).contains(&normalized_query))
        })
        .map(|definition| TagSuggestion {
            id: definition.id.to_string(),
            label: definition.label.to_string(),
            frame: definition.frame.to_string(),
            source: TAG_SOURCE,
        })
        .collect()
}

pub fn kept_tag_labels(clear_tags: &[String]) -> Vec<String> {
    TAGS.iter()
        .filter(|definition| !clear_tags.iter().any(|id| id == definition.id))
        .map(|definition| definition.label.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_id3_and_navidrome_aliases() {
        assert_eq!(canonical_tag_id("COMM"), Some("comment"));
        assert_eq!(canonical_tag_id("Description"), Some("description"));
        assert_eq!(canonical_tag_id("track number"), Some("trackNumber"));
    }

    #[test]
    fn rejects_tags_outside_the_supported_catalog() {
        let error = canonicalize_clear_tags(&["madeUpTag".to_string()]).unwrap_err();
        assert!(error.contains("madeUpTag"));
    }

    #[test]
    fn suggestions_match_labels_and_frames() {
        let suggestions = suggestions("desc");
        assert_eq!(suggestions[0].id, "description");
        assert!(suggestions[0].frame.contains("DESCRIPTION"));
    }

    #[test]
    fn kept_labels_exclude_cleared_ids() {
        let kept = kept_tag_labels(&["comment".to_string()]);
        assert!(!kept.iter().any(|label| label == "Comment"));
        assert!(kept.iter().any(|label| label == "Description"));
    }
}
