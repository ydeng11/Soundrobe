/**
 * Structured output schemas for LLM responses.
 * Ported from Python auto_tagger.llm.schemas.
 */

// ── Candidate Selection ─────────────────────────────────────────────

export interface CandidateSelectionResponse {
  selectedIndex: number | null;
  confidence: number; // 0.0 – 1.0
  reason: string;
}

export function validateSelectionIndex(
  response: CandidateSelectionResponse,
  candidateCount: number,
): void {
  if (
    response.selectedIndex != null &&
    (response.selectedIndex < 0 || response.selectedIndex >= candidateCount)
  ) {
    throw new Error(
      `selectedIndex ${response.selectedIndex} out of range for ${candidateCount} candidates`,
    );
  }
}

// ── Generated Track Tags ────────────────────────────────────────────

export interface GeneratedTrackTags {
  title: string;
  artist?: string | null;
  artists?: string[];
  album?: string | null;
  albumArtist?: string | null;
  trackNumber?: number | null;
  discNumber?: number | null;
}

// ── Fallback Tag Response ───────────────────────────────────────────

export interface FallbackTagResponse {
  artist: string;
  artists: string[];
  album: string;
  albumArtist: string;
  albumArtists: string[];
  year?: string | null;
  genre?: string | null;
  tracks: GeneratedTrackTags[];
  confidence: number; // 0.0 – 1.0
  reason: string;
}

// ── Folder Extraction Response ──────────────────────────────────────

export interface FolderExtractionResponse {
  artist?: string | null;
  album?: string | null;
  year?: string | null;
  disc?: string | null;
}

// ── Genre Enrichment Response ───────────────────────────────────────

export interface GenreEnrichmentResponse {
  genre?: string | null;
}

// ── Audit Types ────────────────────────────────────────────────────

export interface AuditTrackResult {
  index: number;
  field: string;
  status: "correct" | "warning" | "error";
  message: string;
  suggestion?: string | null;
  corrected?: CorrectedTrack | null;
  source?: "deterministic" | "llm";
  confidence?: number;
  autoFixEligible?: boolean;
  autoFixed?: boolean;
}

export interface CorrectedTrack {
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

export interface AuditResponse {
  tracks: AuditTrackResult[];
}
