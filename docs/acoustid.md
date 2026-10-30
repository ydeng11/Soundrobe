# AcoustID audio recognition

AcoustID is an optional fallback when normal MusicBrainz/Discogs matching cannot identify an album. MusicBrainz supplies the resulting metadata. Existing accepted provider matches take precedence.

## Setup

Install Chromaprint's `fpcalc` on the host running Soundrobe (`brew install chromaprint` on macOS, `apt install libchromaprint-tools` on Debian/Ubuntu). The Docker image includes it. Windows users can install the official Chromaprint tools and add their directory to PATH. A custom executable can be selected with `AUTO_TAG_FPCALC_PATH`.

Register an application at [AcoustID](https://acoustid.org/webservice) and use its application/client key, rather than a fingerprint-submission user key. In browser Tag sources or desktop AI & Providers, enter the application key and enable AcoustID recognition. Online metadata lookup must remain enabled.

Alternatively, put `AUTO_TAG_ACOUSTID_ENABLED=true` and `AUTO_TAG_ACOUSTID_API_KEY=your-application-key` in `.env.local` for commands launched through `just`, or the service's environment. Saved settings use `acoustid_enabled` and `acoustid_api_key` in the existing config.yaml. Keys remain native and are masked in renderer responses. Blank or masked values preserve a saved key.

## Behavior

Run the existing album auto-tag action. Soundrobe fingerprints each file and sends its fingerprint and whole-file duration to AcoustID; it does not upload audio. Requests are paced across album jobs. MusicBrainz release details must establish a unique, complete recording map before tags are written through the normal write queue.

Shuffled filenames can be mapped to their correct track positions. Embedded IDs and positions, album/artist hints, edition evidence, durations and individual artist credits remain protected. Partial albums, repeated ambiguous recordings, weak or tied matches, and multiple indistinguishable releases require review. No fingerprint approval is restored from cache.

Missing keys or fpcalc, service errors, exhausted bounds and incomplete evidence are reported without treating them as a confirmed no-match or authorizing AI fallback. Live recognition requires a valid application key; offline fixture tests do not establish live-service availability.
