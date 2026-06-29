import { splitArtistNames } from "../handlers/candidates";
import { cleanAlbumFolderName, extractYearFromName } from "../handlers/fallback";

export type AuditFindingSource = "deterministic" | "llm";
export type AuditFindingStatus = "correct" | "warning" | "error";

export type AuditCoreField =
  | "title"
  | "artist"
  | "artists"
  | "album"
  | "albumArtist"
  | "albumArtists"
  | "year"
  | "genre"
  | "trackNumber"
  | "trackTotal"
  | "discNumber"
  | "discTotal";

export interface AuditCorrectedFields {
  title?: string | null;
  artist?: string | null;
  artists?: string[] | null;
  album?: string | null;
  albumArtist?: string | null;
  albumArtists?: string[] | null;
  year?: string | null;
  genre?: string | null;
  trackNumber?: number | null;
  trackTotal?: number | null;
  discNumber?: number | null;
  discTotal?: number | null;
}

export interface AuditTrackMeta {
  title: string | null;
  artist: string | null;
  artists: string[];
  album: string | null;
  albumArtist: string | null;
  albumArtists?: string[];
  year: string | null;
  genre: string | null;
  trackNumber: number | null;
  trackTotal: number | null;
  discNumber: number | null;
  discTotal: number | null;
}

export interface AuditFinding {
  index: number;
  field: AuditCoreField | string;
  status: AuditFindingStatus;
  message: string;
  suggestion?: string | null;
  corrected?: AuditCorrectedFields | null;
  source?: AuditFindingSource;
  confidence?: number;
  autoFixEligible?: boolean;
  autoFixed?: boolean;
}

export interface AuditReviewTarget {
  index: number;
  field: AuditCoreField;
  current: string;
  expected?: string;
  evidence: string;
  reason: string;
  suggestedCorrection?: AuditCorrectedFields;
}

const DETERMINISTIC_CONFIDENCE = 0.98;
export const LLM_AUTO_FIX_CONFIDENCE = 0.92;

const AMBIGUOUS_ARTIST_DELIMITER_RE = /[,/]/;
const HIGH_CONFIDENCE_ARTIST_DELIMITER_RE =
  /\s+(?:feat\.?|ft\.?|featuring)\s+|[&;；、，＋+\uFF0B]|(?<=[\u4e00-\u9fff\u3400-\u4dbf])[·‧\u00B7.](?=[\u4e00-\u9fff\u3400-\u4dbf])/i;

function compact(value: string | null | undefined): string {
  return (value ?? "").normalize("NFKC").trim().replace(/\s+/g, " ");
}

function comparable(value: string | number | null | undefined): string {
  return compact(value == null ? "" : String(value))
    .toLocaleLowerCase()
    .replace(/[._:：'"`()[\]{}《》「」【】]/g, "")
    .replace(/[‐-‒–—―]+/g, "-")
    .replace(/\s+/g, " ")
    .trim();
}

function sameText(a: string | number | null | undefined, b: string | number | null | undefined): boolean {
  return comparable(a) === comparable(b);
}

function sameStringList(a: string[] | null | undefined, b: string[] | null | undefined): boolean {
  const left = (a ?? []).map(comparable).filter(Boolean);
  const right = (b ?? []).map(comparable).filter(Boolean);
  if (left.length !== right.length) return false;
  return left.every((value, index) => value === right[index]);
}

function makeFinding(
  index: number,
  field: AuditCoreField,
  status: AuditFindingStatus,
  message: string,
  corrected: AuditCorrectedFields | null,
  suggestion: string | null = null,
  autoFixEligible = status === "error" && corrected != null,
): AuditFinding {
  return {
    index,
    field,
    status,
    message,
    suggestion,
    corrected,
    source: "deterministic",
    confidence: DETERMINISTIC_CONFIDENCE,
    autoFixEligible,
    autoFixed: false,
  };
}

function filenameStem(filename: string): string {
  return filename.replace(/\.[^.]+$/, "").trim();
}

interface FilenameFacts {
  title: string | null;
  artist: string | null;
  trackNumber: number | null;
  discNumber: number | null;
}

function parseFilenameFacts(filename: string): FilenameFacts {
  let stem = filenameStem(filename);
  let trackNumber: number | null = null;
  let discNumber: number | null = null;

  const discTrack = /^(\d{1,2})[-_](\d{1,3})(?:\s*[.\-_\s]+\s*|\s+)(.+)$/.exec(stem);
  if (discTrack) {
    discNumber = Number(discTrack[1]);
    trackNumber = Number(discTrack[2]);
    stem = discTrack[3].trim();
  } else {
    const track = /^(\d{1,3})(?:\s*[.)_-]\s*|\s+)(.+)$/.exec(stem);
    if (track) {
      trackNumber = Number(track[1]);
      stem = track[2].trim();
    }
  }

  const artistTitle = /\s[-–—]\s/.exec(stem);
  if (artistTitle && artistTitle.index > 0) {
    const artist = stem.slice(0, artistTitle.index).trim();
    const title = stem.slice(artistTitle.index + artistTitle[0].length).trim();
    return {
      artist: artist || null,
      title: title || null,
      trackNumber,
      discNumber,
    };
  }

  return {
    artist: null,
    title: stem || null,
    trackNumber,
    discNumber,
  };
}

function parseDiscFromFolder(albumHint: string | null | undefined): number | null {
  if (!albumHint) return null;
  const match = /^(?:cd|disc|disk)\s*(\d{1,2})$/i.exec(albumHint.trim());
  return match ? Number(match[1]) : null;
}

function addUniqueFinding(findings: AuditFinding[], finding: AuditFinding): void {
  const exists = findings.some((f) => f.index === finding.index && f.field === finding.field);
  if (!exists) findings.push(finding);
}

function clearYear(albumHint: string | null | undefined): string | null {
  if (!albumHint) return null;
  const year = extractYearFromName(albumHint);
  return year && /^(?:19|20)\d{2}$/.test(year) ? year : null;
}

function shouldSkipMissingAlbumArtist(track: AuditTrackMeta, artistHint: string): boolean {
  return !track.albumArtist && sameText(track.artist, artistHint);
}

function classifyArtistSplit(value: string | null | undefined): "none" | "high" | "ambiguous" {
  if (!value) return "none";
  if (HIGH_CONFIDENCE_ARTIST_DELIMITER_RE.test(value)) return "high";
  if (AMBIGUOUS_ARTIST_DELIMITER_RE.test(value)) return "ambiguous";
  return "none";
}

function addArtistsFinding(
  findings: AuditFinding[],
  index: number,
  sourceArtist: string | null,
  currentArtists: string[],
  canReplaceStaleSingleArtist = false,
): void {
  const splitKind = classifyArtistSplit(sourceArtist);
  if (splitKind === "none") return;

  const expectedArtists = splitArtistNames([sourceArtist]);
  if (expectedArtists.length < 2) return;
  if (sameStringList(currentArtists, expectedArtists)) return;

  const currentIsMissingOrUnsplit =
    currentArtists.length === 0 ||
    (currentArtists.length === 1 && sameText(currentArtists[0], sourceArtist)) ||
    (currentArtists.length === 1 && canReplaceStaleSingleArtist);

  if (splitKind === "high" && currentIsMissingOrUnsplit) {
    addUniqueFinding(findings, makeFinding(
      index,
      "artists",
      "error",
      `Artists should be split into separate values: ${expectedArtists.join(", ")}`,
      { artists: expectedArtists },
      expectedArtists.join(", "),
      true,
    ));
    return;
  }

  addUniqueFinding(findings, makeFinding(
    index,
    "artists",
    "warning",
    `Artists may need manual splitting: ${sourceArtist ?? ""}`,
    null,
    expectedArtists.join(", "),
    false,
  ));
}

export function buildDeterministicAuditFindings(
  artistHint: string | null,
  albumHint: string | null,
  tracks: AuditTrackMeta[],
  filenames: string[],
  options: { discFolderHint?: string | null } = {},
): AuditFinding[] {
  const findings: AuditFinding[] = [];
  const expectedAlbum = albumHint ? cleanAlbumFolderName(albumHint, artistHint).trim() : "";
  const expectedYear = clearYear(albumHint);
  const expectedAlbumArtist = compact(artistHint);
  const expectedAlbumArtists = expectedAlbumArtist ? splitArtistNames([expectedAlbumArtist]) : [];
  const folderDiscNumber = parseDiscFromFolder(options.discFolderHint ?? albumHint);
  const totalTracks = tracks.length > 1 ? tracks.length : null;

  tracks.forEach((track, index) => {
    const filename = filenames[index] ?? "";
    const facts = parseFilenameFacts(filename);

    if (expectedAlbum && !sameText(track.album, expectedAlbum)) {
      addUniqueFinding(findings, makeFinding(
        index,
        "album",
        "error",
        `Album tag does not match album folder "${expectedAlbum}".`,
        { album: expectedAlbum },
        expectedAlbum,
      ));
    }

    if (expectedAlbumArtist && !shouldSkipMissingAlbumArtist(track, expectedAlbumArtist)) {
      if (!sameText(track.albumArtist, expectedAlbumArtist)) {
        addUniqueFinding(findings, makeFinding(
          index,
          "albumArtist",
          "error",
          `Album artist does not match artist folder "${expectedAlbumArtist}".`,
          { albumArtist: expectedAlbumArtist },
          expectedAlbumArtist,
        ));
      }

      const currentAlbumArtists = track.albumArtists ?? (track.albumArtist ? [track.albumArtist] : []);
      if (currentAlbumArtists.length > 0 && expectedAlbumArtists.length > 0 && !sameStringList(currentAlbumArtists, expectedAlbumArtists)) {
        addUniqueFinding(findings, makeFinding(
          index,
          "albumArtists",
          "error",
          `Album artists list does not match artist folder "${expectedAlbumArtist}".`,
          { albumArtists: expectedAlbumArtists },
          expectedAlbumArtists.join(", "),
        ));
      }
    }

    if (facts.title && !sameText(track.title, facts.title)) {
      addUniqueFinding(findings, makeFinding(
        index,
        "title",
        "error",
        `Title does not match filename "${filename}".`,
        { title: facts.title },
        facts.title,
      ));
    }

    if (facts.artist && !sameText(track.artist, facts.artist)) {
      addUniqueFinding(findings, makeFinding(
        index,
        "artist",
        "error",
        `Artist does not match filename artist "${facts.artist}".`,
        { artist: facts.artist },
        facts.artist,
      ));
    }

    addArtistsFinding(
      findings,
      index,
      facts.artist ?? track.artist,
      track.artists ?? [],
      Boolean(facts.artist && track.artist && (track.artists ?? []).length === 1 && sameText((track.artists ?? [])[0], track.artist)),
    );

    if (expectedYear && !sameText(track.year, expectedYear)) {
      addUniqueFinding(findings, makeFinding(
        index,
        "year",
        "error",
        `Year tag does not match clear folder year "${expectedYear}".`,
        { year: expectedYear },
        expectedYear,
      ));
    }

    if (facts.trackNumber != null && track.trackNumber !== facts.trackNumber) {
      addUniqueFinding(findings, makeFinding(
        index,
        "trackNumber",
        "error",
        `Track number does not match filename number ${facts.trackNumber}.`,
        {
          trackNumber: facts.trackNumber,
          ...(totalTracks != null ? { trackTotal: totalTracks } : {}),
        },
        String(facts.trackNumber),
      ));
    } else if (facts.trackNumber != null && totalTracks != null && track.trackTotal !== totalTracks) {
      addUniqueFinding(findings, makeFinding(
        index,
        "trackTotal",
        "error",
        `Track total should be ${totalTracks} for this album.`,
        { trackTotal: totalTracks },
        String(totalTracks),
      ));
    }

    const expectedDiscNumber = facts.discNumber ?? folderDiscNumber;
    if (expectedDiscNumber != null && track.discNumber !== expectedDiscNumber) {
      addUniqueFinding(findings, makeFinding(
        index,
        "discNumber",
        "error",
        `Disc number does not match filename or disc folder number ${expectedDiscNumber}.`,
        { discNumber: expectedDiscNumber },
        String(expectedDiscNumber),
      ));
    }
  });

  return findings;
}

function targetKey(index: number, field: string): string {
  return `${index}:${field}`;
}

function currentValue(track: AuditTrackMeta, field: AuditCoreField): string {
  const value = track[field as keyof AuditTrackMeta];
  if (Array.isArray(value)) return value.join(", ");
  return value == null ? "" : String(value);
}

export function buildLlmReviewTargets(
  tracks: AuditTrackMeta[],
  filenames: string[],
  findings: AuditFinding[],
): AuditReviewTarget[] {
  const targets: AuditReviewTarget[] = [];
  const seen = new Set<string>();

  for (const finding of findings) {
    if (finding.autoFixEligible) continue;
    const field = finding.field as AuditCoreField;
    const key = targetKey(finding.index, field);
    if (seen.has(key)) continue;
    seen.add(key);
    targets.push({
      index: finding.index,
      field,
      current: currentValue(tracks[finding.index], field),
      expected: finding.suggestion ?? undefined,
      evidence: finding.message,
      reason: "Deterministic audit found an ambiguous issue that needs semantic review.",
      suggestedCorrection: finding.corrected ?? undefined,
    });
  }

  const nonBlankGenres = tracks
    .map((track) => compact(track.genre))
    .filter(Boolean);
  const genreSet = new Set(nonBlankGenres.map(comparable));
  const shouldReviewGenre = genreSet.size > 1;

  tracks.forEach((track, index) => {
    const key = targetKey(index, "genre");
    if (seen.has(key)) return;
    if (track.genre && !shouldReviewGenre) return;
    seen.add(key);
    targets.push({
      index,
      field: "genre",
      current: track.genre ?? "",
      evidence: track.genre
        ? `Genre "${track.genre}" differs from other album genre tags.`
        : "Genre tag is empty.",
      reason: "Genre is semantic and should be reviewed by the LLM rather than deterministic rules.",
    });
  });

  return targets;
}

function allowedCorrectionKeys(field: string): Set<keyof AuditCorrectedFields> {
  switch (field) {
    case "album_artist":
      return new Set(["albumArtist"]);
    default:
      return new Set([field as keyof AuditCorrectedFields]);
  }
}

function pickCorrectionForField(
  corrected: AuditCorrectedFields | null | undefined,
  field: string,
): AuditCorrectedFields | null {
  if (!corrected) return null;
  const allowed = allowedCorrectionKeys(field);
  const picked: AuditCorrectedFields = {};
  for (const key of allowed) {
    if (corrected[key] !== undefined) {
      (picked as Record<string, unknown>)[key] = corrected[key];
    }
  }
  return Object.keys(picked).length > 0 ? picked : null;
}

function correctionOnlyTouchesField(
  corrected: AuditCorrectedFields | null | undefined,
  field: string,
): boolean {
  if (!corrected) return false;
  const allowed = allowedCorrectionKeys(field);
  return Object.entries(corrected).every(([key, value]) => value === undefined || allowed.has(key as keyof AuditCorrectedFields));
}

function clampConfidence(value: unknown): number {
  if (typeof value !== "number" || Number.isNaN(value)) return 0;
  return Math.max(0, Math.min(1, value));
}

export function normalizeLlmAuditResults(
  results: AuditFinding[],
  reviewTargets: AuditReviewTarget[],
): AuditFinding[] {
  const targetKeys = new Set(reviewTargets.map((target) => targetKey(target.index, target.field)));

  return results
    .filter((result) => result.status !== "correct")
    .map((result) => {
      const confidence = clampConfidence(result.confidence);
      const key = targetKey(result.index, result.field);
      const hasMatchingTarget = targetKeys.has(key);
      const corrected = pickCorrectionForField(result.corrected, result.field);
      const onlyTouchesTarget = correctionOnlyTouchesField(result.corrected, result.field);
      const autoFixEligible =
        hasMatchingTarget &&
        onlyTouchesTarget &&
        result.status === "error" &&
        confidence >= LLM_AUTO_FIX_CONFIDENCE &&
        corrected != null;

      return {
        ...result,
        corrected,
        source: "llm" as const,
        confidence,
        autoFixEligible,
        autoFixed: false,
      };
    });
}

export function mergeAuditFindings(
  deterministicFindings: AuditFinding[],
  llmFindings: AuditFinding[],
): AuditFinding[] {
  const llmKeys = new Set(llmFindings.map((finding) => targetKey(finding.index, finding.field)));
  return [
    ...deterministicFindings.filter((finding) => finding.autoFixEligible || !llmKeys.has(targetKey(finding.index, finding.field))),
    ...llmFindings,
  ];
}
