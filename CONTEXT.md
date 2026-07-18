# Auto Tagging Context

This context describes the language used to reason about album matching and tag
writing decisions in Soundrobe.

## Language

**Lookup Hint**:
A human-, folder-, tag-, filename-, or model-derived value used to search for or
rank candidate releases. A lookup hint is not enough by itself to write final
track metadata.
_Avoid_: Final tag, authority

**Track Evidence**:
A per-file signal that can align a local audio file to a specific provider
track. Track evidence explains why a local file and remote track are believed
to be the same recording or track entry.
_Avoid_: Guess, fallback

**Write Authority**:
The level of trust required for a source to write a specific tag field to disk.
Different fields can require different write authority.
_Avoid_: Lookup hint, candidate

**Tag Noise**:
Existing file metadata that conflicts with coherent folder, filename, manual, or
provider evidence and should not guide release search or final writes.
_Avoid_: Existing tag, local truth

**Unattended Auto-Tagging**:
The default mode where Soundrobe proceeds without asking the user to resolve
uncertainty during matching. It should tag what it can justify and leave the
remaining uncertainty for later correction.
_Avoid_: Interactive tagging, manual review flow

**Release Confidence**:
A cross-source judgment that a provider candidate is the intended album release.
It comes from agreement between lookup hints, file-derived track evidence, model
fallbacks, and provider response data rather than from any single source.
_Avoid_: LLM confidence, album-title match

**Release Acceptance**:
The decision that a provider candidate is reliable enough to participate in
auto-tagging. Release acceptance can choose album-level values, but per-track
fields still require track evidence.
_Avoid_: Track match, write authority

**Name Agreement**:
Agreement between local, manual, model, and provider names for the artist,
album, or track. For artist-scoped release matching, album and track name
agreement are the primary signals.
_Avoid_: Count match, position match

**Release Shortlist**:
A small set of provider releases that pass album name agreement during
artist-scoped browsing and are worth fetching in detail. The final release
choice can use track name agreement across this shortlist.
_Avoid_: First match, all releases

**Shortlist Filter**:
The album name agreement rule that a provider release must pass before entering
the release shortlist. The shortlist cap limits work after filtering; it is not
the reason a weak name match is accepted.
_Avoid_: Cap, broad search

**Fallback Track Name**:
A track title or artist inferred from local file context when provider evidence
is not yet available. Folder and model-derived fallback track names have the
same weight because both are cleanup interpretations of local filenames.
_Avoid_: Provider track, authoritative title

**Model-Assisted Extraction**:
Use of the model to extract meaningful artist, album, track, or genre values
from local file context. Outside genre, model output is local-context
interpretation rather than external music knowledge.
_Avoid_: Model authority, provider evidence

**Unique Track Alignment**:
The rule that one local file can align to one provider track only when the
matching evidence identifies a single provider track for that file.
_Avoid_: Multiple match, ambiguous match
