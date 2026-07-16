//! Deterministic local-to-provider track alignment.
//!
//! Provider order is not file order. This module establishes one-to-one track
//! identity from recording IDs, normalized tag/filename titles, and duration
//! evidence before any provider per-track fields may reach the writer.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;

use super::auto_tag::TrackCandidate;
use crate::state::providers::convert_chinese_text;

const ANNOTATION_KEYWORDS: &str =
    r"live|remaster(?:ed)?|version|karaoke|instrumental|伴奏|现场|現場|片头|片頭|片尾|theme";
const TITLE_POLLUTION_EXTRA_MIN_CHARS: usize = 3;
const TITLE_POLLUTION_EXTRA_MIN_RATIO: f64 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchEvidence {
    MusicbrainzTrackId,
    TagTitle,
    FilenameTitle,
    FallbackTitle,
    ContainedTitle,
    Position,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipKind {
    NoTitleMatch,
    DurationMismatch,
    DuplicateAmbiguous,
    NoLocalEvidence,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkipReason {
    pub local_index: usize,
    pub kind: SkipKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MatchStats {
    pub matched: usize,
    pub local: usize,
    pub remote: usize,
    pub skipped: Vec<SkipReason>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MatchedCandidate {
    pub tracks: Vec<TrackCandidate>,
    pub stats: MatchStats,
    pub evidence: Vec<Option<MatchEvidence>>,
    pub is_full_ordered_match: bool,
}

#[derive(Default)]
struct LocalForms {
    tag: Vec<String>,
    filename: Vec<String>,
    fallback: Vec<String>,
    filename_raw: String,
}

struct RemoteMeta {
    primary: String,
    variants: Vec<String>,
    primary_forms: HashSet<String>,
    duration: Option<f64>,
}

pub fn match_remote_candidate_tracks(
    local_tracks: &[TrackCandidate],
    filenames: &[String],
    remote_tracks: &[TrackCandidate],
    source: &str,
    artist_hints: &[String],
    alternate_titles: &[Option<String>],
) -> MatchedCandidate {
    let mut stats = MatchStats {
        matched: 0,
        local: local_tracks.len(),
        remote: remote_tracks.len(),
        skipped: Vec::new(),
    };
    if local_tracks.is_empty() || remote_tracks.is_empty() {
        return MatchedCandidate {
            tracks: local_tracks.to_vec(),
            evidence: vec![None; local_tracks.len()],
            stats,
            is_full_ordered_match: false,
        };
    }

    let local_forms = local_tracks
        .iter()
        .enumerate()
        .map(|(index, track)| {
            let mut artists = artist_hints.to_vec();
            artists.extend(track.artist.iter().cloned());
            artists.extend(track.artists.iter().cloned());
            let filename = filenames.get(index).cloned().unwrap_or_default();
            let (tag, filename_forms) = title_forms(track.title.as_deref(), &filename, &artists);
            let fallback = alternate_titles
                .get(index)
                .and_then(Option::as_deref)
                .map(|title| title_forms(Some(title), "", &artists).0)
                .unwrap_or_default();
            LocalForms {
                tag,
                filename: filename_forms,
                fallback,
                filename_raw: filename,
            }
        })
        .collect::<Vec<_>>();
    let remote_meta = remote_tracks
        .iter()
        .map(|track| {
            let primary = strip_annotations(track.title.as_deref().unwrap_or_default());
            let primary_variants = remote_title_variants(track.title.iter().map(String::as_str));
            let variants = remote_title_variants(
                track
                    .title
                    .iter()
                    .chain(track.match_titles.iter())
                    .map(String::as_str),
            );
            RemoteMeta {
                primary,
                primary_forms: primary_variants
                    .iter()
                    .map(|title| normalize_title(title))
                    .filter(|title| !title.is_empty())
                    .collect(),
                variants,
                duration: normalize_duration(track.length, source),
            }
        })
        .collect::<Vec<_>>();

    let mut remote_title_index: HashMap<String, Vec<usize>> = HashMap::new();
    let mut remote_id_index: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, track) in remote_tracks.iter().enumerate() {
        if let Some(id) = track.musicbrainz_track_id.as_deref() {
            remote_id_index.entry(id).or_default().push(index);
        }
        for variant in &remote_meta[index].variants {
            let normalized = normalize_title(variant);
            if normalized.is_empty() {
                continue;
            }
            let indexes = remote_title_index.entry(normalized).or_default();
            if !indexes.contains(&index) {
                indexes.push(index);
            }
        }
    }

    let mut matched_remote = HashSet::new();
    let mut matched_local = vec![None; local_tracks.len()];
    let mut evidence = vec![None; local_tracks.len()];
    let mut alternate_remote_title = vec![false; local_tracks.len()];

    for (local_index, local_track) in local_tracks.iter().enumerate() {
        if source == "musicbrainz" {
            if let Some(id) = local_track.musicbrainz_track_id.as_deref() {
                let available = remote_id_index
                    .get(id)
                    .into_iter()
                    .flatten()
                    .copied()
                    .filter(|index| !matched_remote.contains(index))
                    .collect::<Vec<_>>();
                if available.len() == 1 {
                    accept_match(
                        local_index,
                        available[0],
                        MatchEvidence::MusicbrainzTrackId,
                        &mut matched_local,
                        &mut matched_remote,
                        &mut evidence,
                        &mut stats,
                    );
                    continue;
                }
                if available.len() > 1 {
                    stats.skipped.push(SkipReason {
                        local_index,
                        kind: SkipKind::DuplicateAmbiguous,
                    });
                    continue;
                }
            }
        }

        let forms = &local_forms[local_index];
        let ordered_forms = forms
            .tag
            .iter()
            .map(|form| (form, MatchEvidence::TagTitle))
            .chain(
                forms
                    .filename
                    .iter()
                    .map(|form| (form, MatchEvidence::FilenameTitle)),
            )
            .chain(
                forms
                    .fallback
                    .iter()
                    .map(|form| (form, MatchEvidence::FallbackTitle)),
            )
            .collect::<Vec<_>>();
        let local_duration = normalize_duration(local_track.length, "local");
        let mut matched = false;
        let mut saw_duration_mismatch = false;
        let mut saw_ambiguity = false;

        for (form, form_evidence) in &ordered_forms {
            let available = remote_title_index
                .get(*form)
                .into_iter()
                .flatten()
                .copied()
                .filter(|index| !matched_remote.contains(index))
                .collect::<Vec<_>>();
            if available.is_empty() {
                continue;
            }
            let chosen = choose_by_duration(&available, local_duration, &remote_meta);
            let Some(remote_index) = chosen else {
                if available.len() == 1 && local_duration.is_some() {
                    saw_duration_mismatch = true;
                } else {
                    saw_ambiguity = true;
                }
                continue;
            };
            alternate_remote_title[local_index] =
                !remote_meta[remote_index].primary_forms.contains(*form);
            accept_match(
                local_index,
                remote_index,
                *form_evidence,
                &mut matched_local,
                &mut matched_remote,
                &mut evidence,
                &mut stats,
            );
            matched = true;
            break;
        }

        if !matched && is_api_title_source(source) {
            let contained = remote_meta
                .iter()
                .enumerate()
                .filter(|(index, _)| !matched_remote.contains(index))
                .filter(|(_, remote)| {
                    replacement_title(
                        local_track.title.as_deref(),
                        Some(&remote.primary),
                        &remote.variants,
                    )
                    .is_some()
                })
                .filter(|(_, remote)| match (local_duration, remote.duration) {
                    (Some(local), Some(remote)) => durations_match(local, remote),
                    _ => true,
                })
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            if contained.len() == 1 {
                accept_match(
                    local_index,
                    contained[0],
                    MatchEvidence::ContainedTitle,
                    &mut matched_local,
                    &mut matched_remote,
                    &mut evidence,
                    &mut stats,
                );
                matched = true;
            } else if contained.len() > 1 {
                saw_ambiguity = true;
            }
        }

        if !matched {
            stats.skipped.push(SkipReason {
                local_index,
                kind: if saw_ambiguity {
                    SkipKind::DuplicateAmbiguous
                } else if saw_duration_mismatch {
                    SkipKind::DurationMismatch
                } else if ordered_forms.is_empty() {
                    SkipKind::NoLocalEvidence
                } else {
                    SkipKind::NoTitleMatch
                },
            });
        }
    }

    if stats.matched == 0 && local_tracks.len() == remote_tracks.len() && local_tracks.len() >= 2 {
        for index in 0..local_tracks.len() {
            matched_local[index] = Some(index);
            matched_remote.insert(index);
            evidence[index] = Some(MatchEvidence::Position);
        }
        stats.matched = local_tracks.len();
        stats.skipped.clear();
    }

    let full_match =
        stats.matched == local_tracks.len() && local_tracks.len() == remote_tracks.len();
    let tracks = local_tracks
        .iter()
        .enumerate()
        .map(|(local_index, local)| {
            let Some(remote_index) = matched_local[local_index] else {
                return local.clone();
            };
            aligned_track(
                local,
                &remote_tracks[remote_index],
                &remote_meta[remote_index],
                evidence[local_index],
                alternate_remote_title[local_index],
                &local_forms[local_index],
                source,
                artist_hints,
                full_match,
            )
        })
        .collect();

    MatchedCandidate {
        tracks,
        stats,
        evidence,
        is_full_ordered_match: full_match,
    }
}

fn accept_match(
    local: usize,
    remote: usize,
    why: MatchEvidence,
    matched_local: &mut [Option<usize>],
    matched_remote: &mut HashSet<usize>,
    evidence: &mut [Option<MatchEvidence>],
    stats: &mut MatchStats,
) {
    matched_local[local] = Some(remote);
    matched_remote.insert(remote);
    evidence[local] = Some(why);
    stats.matched += 1;
}

fn choose_by_duration(
    candidates: &[usize],
    local_duration: Option<f64>,
    remote: &[RemoteMeta],
) -> Option<usize> {
    if candidates.len() == 1 {
        let candidate = candidates[0];
        return match (local_duration, remote[candidate].duration) {
            (Some(local), Some(remote)) if !durations_match(local, remote) => None,
            _ => Some(candidate),
        };
    }
    let local_duration = local_duration?;
    let matches = candidates
        .iter()
        .copied()
        .filter(|index| {
            remote[*index]
                .duration
                .is_some_and(|remote| durations_match(local_duration, remote))
        })
        .collect::<Vec<_>>();
    (matches.len() == 1).then_some(matches[0])
}

#[allow(clippy::too_many_arguments)]
fn aligned_track(
    local: &TrackCandidate,
    remote: &TrackCandidate,
    remote_meta: &RemoteMeta,
    evidence: Option<MatchEvidence>,
    matched_alternate_title: bool,
    forms: &LocalForms,
    source: &str,
    artist_hints: &[String],
    full_match: bool,
) -> TrackCandidate {
    let mut result = local.clone();
    let api_source = is_api_title_source(source);
    let title_replacement = api_source
        .then(|| {
            replacement_title(
                local.title.as_deref(),
                remote.title.as_deref(),
                &remote_meta.variants,
            )
        })
        .flatten();
    let tag_matched_primary = forms
        .tag
        .iter()
        .any(|form| remote_meta.primary_forms.contains(form));

    result.title = if api_source && matched_alternate_title {
        remote.title.clone().or_else(|| local.title.clone())
    } else if title_replacement.is_some() {
        title_replacement
    } else if api_source && evidence == Some(MatchEvidence::MusicbrainzTrackId) {
        remote.title.clone().or_else(|| local.title.clone())
    } else if api_source && is_placeholder(local.title.as_deref()) {
        remote.title.clone().or_else(|| local.title.clone())
    } else if evidence == Some(MatchEvidence::TagTitle) && tag_matched_primary {
        local.title.clone()
    } else if evidence == Some(MatchEvidence::FilenameTitle) {
        let mut artists = artist_hints.to_vec();
        artists.extend(local.artist.iter().cloned());
        artists.extend(local.artists.iter().cloned());
        clean_filename_title(&forms.filename_raw, &artists).or_else(|| local.title.clone())
    } else if api_source && evidence == Some(MatchEvidence::FallbackTitle) {
        remote.title.clone().or_else(|| local.title.clone())
    } else {
        local.title.clone()
    };

    if remote.musicbrainz_track_id.is_some() {
        result.musicbrainz_track_id = remote.musicbrainz_track_id.clone();
    }
    let strong_artist_evidence = matches!(
        evidence,
        Some(
            MatchEvidence::MusicbrainzTrackId
                | MatchEvidence::TagTitle
                | MatchEvidence::FilenameTitle
                | MatchEvidence::FallbackTitle
                | MatchEvidence::ContainedTitle
        )
    );
    let local_artist_blank = local
        .artist
        .as_deref()
        .is_none_or(|artist| artist.trim().is_empty());
    if api_source
        && remote.artist.is_some()
        && (strong_artist_evidence
            || (evidence == Some(MatchEvidence::Position) && local_artist_blank))
    {
        result.artist = remote.artist.clone();
        result.artists = if remote.artists.is_empty() {
            remote.artist.iter().cloned().collect()
        } else {
            remote.artists.clone()
        };
    }
    if full_match {
        result.track_number = remote.track_number.or(local.track_number);
        result.track_total = remote.track_total.or(local.track_total);
        result.disc_number = remote.disc_number.or(local.disc_number);
        result.disc_total = remote.disc_total.or(local.disc_total);
    }
    result
}

fn title_forms(
    tag_title: Option<&str>,
    filename: &str,
    known_artists: &[String],
) -> (Vec<String>, Vec<String>) {
    let tag = tag_title
        .into_iter()
        .flat_map(|title| {
            let cleaned = strip_annotations(title);
            let mut titles = vec![cleaned.clone()];
            if let Some(stripped) = strip_known_artist_suffix(&cleaned, known_artists) {
                titles.push(stripped);
            }
            titles
        })
        .flat_map(|title| title_variants(&title))
        .collect::<Vec<_>>();
    let filename = clean_filename_title(filename, known_artists)
        .into_iter()
        .flat_map(|title| title_variants(&title))
        .collect::<Vec<_>>();
    (deduplicate(tag), deduplicate(filename))
}

fn title_variants(title: &str) -> Vec<String> {
    let mut variants = Vec::new();
    for component in std::iter::once(title.to_string()).chain(bilingual_components(title)) {
        variants.push(normalize_title(&component));
        variants.push(normalize_title(&convert_chinese_text(
            &component,
            "traditional",
        )));
        variants.push(normalize_title(&convert_chinese_text(
            &component,
            "simplified",
        )));
    }
    deduplicate(variants)
}

fn remote_title_variants<'a>(titles: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut variants = Vec::new();
    for title in titles {
        let cleaned = strip_annotations(title);
        for component in std::iter::once(cleaned).chain(bilingual_components(title)) {
            variants.extend(title_variants(&component));
            if let Some(leading) = leading_cjk(&component) {
                variants.extend(title_variants(&leading));
            }
        }
    }
    deduplicate(variants)
}

fn normalize_title(value: &str) -> String {
    static PUNCTUATION: OnceLock<Regex> = OnceLock::new();
    PUNCTUATION
        .get_or_init(|| Regex::new(r"[\p{P}\p{S}]+").expect("valid title punctuation regex"))
        .replace_all(
            &value
                .nfkc()
                .collect::<String>()
                .to_lowercase()
                .replace('妳', "你"),
            " ",
        )
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn strip_annotations(value: &str) -> String {
    static PAREN: OnceLock<Regex> = OnceLock::new();
    static BRACKET: OnceLock<Regex> = OnceLock::new();
    static TRAILING: OnceLock<Regex> = OnceLock::new();
    let mut value = value.nfkc().collect::<String>().trim().to_string();
    for regex in [
        PAREN.get_or_init(|| {
            Regex::new(&format!(
                r"(?i)[(（][^)]*(?:{ANNOTATION_KEYWORDS})[^)]*[)）]\s*$"
            ))
            .expect("valid title annotation regex")
        }),
        BRACKET.get_or_init(|| {
            Regex::new(&format!(r"(?i)\[[^]]*(?:{ANNOTATION_KEYWORDS})[^]]*\]\s*$"))
                .expect("valid title annotation regex")
        }),
        TRAILING.get_or_init(|| {
            Regex::new(&format!(r"(?i)[\s\-–—]+(?:{ANNOTATION_KEYWORDS})\s*$"))
                .expect("valid title annotation regex")
        }),
    ] {
        value = regex.replace(&value, "").trim().to_string();
    }
    value
}

fn bilingual_components(value: &str) -> impl Iterator<Item = String> + '_ {
    static SEPARATOR: OnceLock<Regex> = OnceLock::new();
    let has_cjk = value.chars().any(is_cjk);
    let has_latin = value
        .chars()
        .any(|character| character.is_ascii_alphabetic());
    SEPARATOR
        .get_or_init(|| {
            Regex::new(r"[()（）]|\s+(?:/|[-–—])\s+").expect("valid bilingual separator regex")
        })
        .split(value)
        .map(str::trim)
        .filter(move |part| {
            has_cjk && has_latin && !part.is_empty() && *part != value && usable_containment(part)
        })
        .map(str::to_string)
}

fn leading_cjk(value: &str) -> Option<String> {
    let leading = value
        .chars()
        .take_while(|character| is_cjk(*character))
        .collect::<String>();
    (!leading.is_empty() && leading != value).then_some(leading)
}

fn usable_containment(value: &str) -> bool {
    if value.chars().any(is_cjk) {
        value
            .chars()
            .filter(|character| !character.is_whitespace())
            .count()
            >= 2
    } else {
        value.split_whitespace().any(|token| token.len() >= 3)
    }
}

fn clean_filename_title(filename: &str, known_artists: &[String]) -> Option<String> {
    static TRACK_PREFIX: OnceLock<Regex> = OnceLock::new();
    let stem = Path::new(filename).file_stem()?.to_str()?.trim();
    let mut title = TRACK_PREFIX
        .get_or_init(|| Regex::new(r"^(\d+)[\s.‐‑‒–—―-]*").expect("valid track prefix regex"))
        .replace(stem, "")
        .trim()
        .to_string();
    if title.is_empty() {
        return None;
    }
    if let Some((artist, remainder)) = split_spaced_artist_prefix(&title) {
        if !artist.is_empty() && !remainder.is_empty() {
            title = remainder.to_string();
        }
    } else if let Some(remainder) = strip_known_artist_prefix(&title, known_artists) {
        title = remainder;
    }
    let title = strip_annotations(&title);
    (!title.is_empty()).then_some(title)
}

fn split_spaced_artist_prefix(value: &str) -> Option<(&str, &str)> {
    for separator in [" - ", " – ", " — "] {
        if let Some(parts) = value.split_once(separator) {
            return Some(parts);
        }
    }
    None
}

fn strip_known_artist_prefix(value: &str, artists: &[String]) -> Option<String> {
    for (index, character) in value.char_indices() {
        if !matches!(character, '-' | '–' | '—') {
            continue;
        }
        let artist = normalize_title(value[..index].trim());
        let title = value[index + character.len_utf8()..].trim();
        if !title.is_empty() && artists.iter().any(|known| normalize_title(known) == artist) {
            return Some(title.to_string());
        }
    }
    None
}

fn strip_known_artist_suffix(value: &str, artists: &[String]) -> Option<String> {
    for (index, character) in value.char_indices() {
        if !matches!(character, '-' | '–' | '—') {
            continue;
        }
        let title = value[..index].trim();
        let artist = normalize_title(value[index + character.len_utf8()..].trim());
        if !title.is_empty() && artists.iter().any(|known| normalize_title(known) == artist) {
            return Some(title.to_string());
        }
    }
    None
}

fn replacement_title(
    current: Option<&str>,
    api_title: Option<&str>,
    api_variants: &[String],
) -> Option<String> {
    let current = current?.trim();
    let api_title = api_title?.trim();
    if current.is_empty() || api_title.is_empty() {
        return None;
    }
    let core = current.split(['(', '（']).next().unwrap_or(current).trim();
    let display_core = [" - ", " – ", " — "]
        .iter()
        .find_map(|separator| core.rsplit_once(separator).map(|(_, title)| title.trim()))
        .unwrap_or(core);
    let suffix = &current[core.len()..];
    let candidates = [
        (current.to_string(), core.to_string()),
        (format!("{display_core}{suffix}"), display_core.to_string()),
    ];
    let mut api_candidates = std::iter::once(api_title.to_string())
        .chain(api_variants.iter().cloned())
        .collect::<Vec<_>>();
    api_candidates = deduplicate(api_candidates);
    let primary = normalize_title(api_title);
    for (candidate, candidate_core) in candidates {
        let normalized = normalize_title(&candidate);
        let normalized_core = normalize_title(&candidate_core);
        for api in &api_candidates {
            let api_normalized = normalize_title(api);
            if should_replace_polluted(&normalized, &api_normalized) {
                return Some(if normalized_core == primary {
                    candidate_core
                } else {
                    api_title.to_string()
                });
            }
        }
    }
    None
}

fn should_replace_polluted(current: &str, api: &str) -> bool {
    if api.len() < 2 || current == api || !current.starts_with(api) {
        return false;
    }
    let extra = current[api.len()..].trim();
    if extra.is_empty()
        || extra.split_whitespace().any(|token| {
            matches!(
                token,
                "live"
                    | "remix"
                    | "demo"
                    | "acoustic"
                    | "instrumental"
                    | "karaoke"
                    | "edit"
                    | "version"
                    | "现场"
                    | "現場"
                    | "伴奏"
            )
        })
    {
        return false;
    }
    let extra_chars = current.chars().count().saturating_sub(api.chars().count());
    let ratio = extra_chars as f64 / current.chars().count().max(1) as f64;
    extra_chars >= TITLE_POLLUTION_EXTRA_MIN_CHARS || ratio >= TITLE_POLLUTION_EXTRA_MIN_RATIO
}

fn is_placeholder(value: Option<&str>) -> bool {
    static PLACEHOLDER: OnceLock<Regex> = OnceLock::new();
    value.is_some_and(|value| {
        PLACEHOLDER
            .get_or_init(|| {
                Regex::new(r"(?i)^\s*track\s*\d{1,3}\s*$").expect("valid placeholder regex")
            })
            .is_match(value)
    })
}

fn normalize_duration(value: Option<f64>, source: &str) -> Option<f64> {
    value.filter(|value| *value > 0.0).map(|value| {
        if source == "musicbrainz" && value > 1000.0 {
            value / 1000.0
        } else {
            value
        }
    })
}

fn durations_match(local: f64, remote: f64) -> bool {
    (local - remote).abs() <= 5.0_f64.max(local * 0.03)
}

fn is_api_title_source(source: &str) -> bool {
    matches!(source, "musicbrainz" | "discogs")
}

fn is_cjk(character: char) -> bool {
    matches!(character as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF)
}

fn deduplicate(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| !value.is_empty() && seen.insert(value.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str) -> TrackCandidate {
        TrackCandidate {
            title: Some(title.to_string()),
            ..TrackCandidate::default()
        }
    }

    #[test]
    fn filename_forms_strip_number_and_known_artist() {
        let (_, forms) = title_forms(None, "05. 费玉清-变色湖长城.flac", &["费玉清".into()]);
        assert!(forms.contains(&"变色湖长城".to_string()));
    }

    #[test]
    fn duplicate_titles_require_unique_duration_evidence() {
        let local = vec![TrackCandidate {
            length: Some(200.0),
            ..track("Song")
        }];
        let remote = vec![
            TrackCandidate {
                length: Some(200_000.0),
                ..track("Song")
            },
            TrackCandidate {
                length: Some(260_000.0),
                ..track("Song")
            },
        ];

        let matched = match_remote_candidate_tracks(&local, &[], &remote, "musicbrainz", &[], &[]);

        assert_eq!(matched.stats.matched, 1);
        assert_eq!(matched.evidence, vec![Some(MatchEvidence::TagTitle)]);
    }

    #[test]
    fn positional_fallback_preserves_titles_and_nonblank_artists() {
        let local = vec![
            TrackCandidate {
                artist: Some("Local".into()),
                artists: vec!["Local".into()],
                ..track("本地一")
            },
            TrackCandidate {
                artist: None,
                ..track("本地二")
            },
        ];
        let remote = vec![
            TrackCandidate {
                artist: Some("Remote One".into()),
                track_number: Some(1),
                ..track("Remote A")
            },
            TrackCandidate {
                artist: Some("Remote Two".into()),
                track_number: Some(2),
                ..track("Remote B")
            },
        ];

        let matched = match_remote_candidate_tracks(&local, &[], &remote, "musicbrainz", &[], &[]);

        assert!(matched.is_full_ordered_match);
        assert_eq!(matched.evidence, vec![Some(MatchEvidence::Position); 2]);
        assert_eq!(matched.tracks[0].title.as_deref(), Some("本地一"));
        assert_eq!(matched.tracks[0].artist.as_deref(), Some("Local"));
        assert_eq!(matched.tracks[1].artist.as_deref(), Some("Remote Two"));
        assert_eq!(matched.tracks[1].track_number, Some(2));
    }

    #[test]
    fn filename_title_is_used_when_the_tag_is_wrong() {
        let local = vec![track("Wrong Title")];
        let remote = vec![track("Song A")];

        let matched = match_remote_candidate_tracks(
            &local,
            &["01. Song A.flac".into()],
            &remote,
            "musicbrainz",
            &[],
            &[],
        );

        assert_eq!(matched.evidence, vec![Some(MatchEvidence::FilenameTitle)]);
        assert_eq!(matched.tracks[0].title.as_deref(), Some("Song A"));
    }

    #[test]
    fn recording_id_is_stronger_than_a_conflicting_title() {
        let local = vec![TrackCandidate {
            musicbrainz_track_id: Some("recording-1".into()),
            ..track("Stale Title")
        }];
        let remote = vec![TrackCandidate {
            musicbrainz_track_id: Some("recording-1".into()),
            ..track("Canonical Title")
        }];

        let matched = match_remote_candidate_tracks(&local, &[], &remote, "musicbrainz", &[], &[]);

        assert_eq!(
            matched.evidence,
            vec![Some(MatchEvidence::MusicbrainzTrackId)]
        );
        assert_eq!(matched.tracks[0].title.as_deref(), Some("Canonical Title"));
    }

    #[test]
    fn simplified_and_traditional_titles_match_without_replacing_local_text() {
        let local = vec![TrackCandidate {
            artist: Some("本地歌手".into()),
            ..track("传奇")
        }];
        let remote = vec![TrackCandidate {
            artist: Some("遠端歌手".into()),
            ..track("傳奇")
        }];

        let matched = match_remote_candidate_tracks(&local, &[], &remote, "musicbrainz", &[], &[]);

        assert_eq!(matched.stats.matched, 1);
        assert_eq!(matched.tracks[0].title.as_deref(), Some("传奇"));
        assert_eq!(matched.tracks[0].artist.as_deref(), Some("遠端歌手"));
    }

    #[test]
    fn provider_title_cleans_suffix_pollution_but_not_meaningful_versions() {
        let polluted = match_remote_candidate_tracks(
            &[track("微光(亚特兰提斯)(24bit-48Hz)")],
            &[],
            &[track("微光")],
            "musicbrainz",
            &[],
            &[],
        );
        let live = match_remote_candidate_tracks(
            &[track("Song (Live)")],
            &[],
            &[track("Song")],
            "musicbrainz",
            &[],
            &[],
        );

        assert_eq!(polluted.tracks[0].title.as_deref(), Some("微光"));
        assert_eq!(live.tracks[0].title.as_deref(), Some("Song (Live)"));
    }

    #[test]
    fn positional_fallback_replaces_placeholder_titles_only() {
        let matched = match_remote_candidate_tracks(
            &[track("Track 01"), track("Track 02")],
            &[],
            &[track("First"), track("Second")],
            "discogs",
            &[],
            &[],
        );

        assert_eq!(matched.tracks[0].title.as_deref(), Some("First"));
        assert_eq!(matched.tracks[1].title.as_deref(), Some("Second"));
    }
}
