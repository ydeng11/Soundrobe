#!/usr/bin/env bash
# ============================================================================
# dsf-to-flac.sh
#
# Converts DSF (DSD) files to FLAC with metadata.
#
# Usage: dsf-to-flac.sh <source_dir> [artist]
#        dsf-to-flac.sh /Volumes/downloads/李克勤/李克勤\ -\ 我著10号\ SACD 李克勤
#
# Requires: ffmpeg
# ============================================================================

set -euo pipefail

if [ $# -lt 1 ]; then
    echo "Usage: $0 <source_dir> [artist]"
    exit 1
fi

SOURCE_DIR="$1"
ARTIST="${2:-$(basename "$(dirname "$SOURCE_DIR")")}"
OUTPUT_BASE="${HOME}/Music/${ARTIST}"
ALBUM_NAME="$(basename "${SOURCE_DIR}")"
ALBUM_OUTPUT="${OUTPUT_BASE}/${ALBUM_NAME}"
LOG_FILE="${HOME}/code/soundrobe/dsf-to-flac.log"

# DSD-to-PCM conversion settings
TARGET_RATE="${TARGET_RATE:-88200}"
LOWPASS_FREQ="${LOWPASS_FREQ:-30000}"
BITS_PER_SAMPLE="${BITS_PER_SAMPLE:-24}"

mkdir -p "${ALBUM_OUTPUT}"

log() { echo "$@" | tee -a "${LOG_FILE}"; }

# Parse txt file for track names (GBK encoded)
TRACK_NAMES=()
parse_txt_file() {
    local txt_file
    txt_file=$(find "${SOURCE_DIR}" -maxdepth 1 -name "*.txt" ! -name "a_*" ! -name "必读*" | head -1)
    if [ -z "${txt_file}" ] || [ ! -f "${txt_file}" ]; then
        log "  No track listing txt found"
        return
    fi

    # Detect encoding and convert to UTF-8 if needed
    local encoding
    encoding=$(file -b --mime-encoding "${txt_file}")

    # Extract tracks - may have 曲目 header or start directly with track numbers
    local in_tracks=0
    while IFS= read -r line; do
        # Strip carriage return
        line="${line%$'\r'}"
        # Match lines like "01. Title" or "01  Title"
        if [[ "${line}" =~ ^([0-9]+)[.\ ]+(.+) ]]; then
            in_tracks=1
            local num=$((10#${BASH_REMATCH[1]}))
            local title="${BASH_REMATCH[2]}"
            # Clean title: remove invalid chars, CR, trim
            title=$(echo "${title}" | tr -d '\r' | sed 's/[\/:<>"|?*]//g' | sed 's/  */ /g' | sed 's/^ //;s/ $//')
            [ -n "${title}" ] && TRACK_NAMES[${num}]="${title}"
        elif [[ "${line}" =~ 曲目 ]]; then
            in_tracks=1
        fi
    done < <(
        if [[ "${encoding}" == *iso-8859* ]] || [[ "${encoding}" == *unknown* ]]; then
            iconv -f GBK -t UTF-8 "${txt_file}" 2>/dev/null
        else
            cat "${txt_file}"
        fi
        echo ""  # Ensure trailing newline for last line
    )

    log "  Loaded ${#TRACK_NAMES[@]} track names from $(basename "${txt_file}")"
}

parse_txt_file

# Precompute DSF files and total track count
shopt -s nullglob
DSF_FILES=("${SOURCE_DIR}"/*.dsf "${SOURCE_DIR}"/*.DSF)
total_tracks=${#DSF_FILES[@]}
shopt -u nullglob

log "============================================================"
log "DSF to FLAC started at $(date)"
log "Source: ${SOURCE_DIR}"
log "Output: ${ALBUM_OUTPUT}"
log "Tracks: ${total_tracks}  Target rate: ${TARGET_RATE} Hz"
log "============================================================"

# Convert each DSF file
errors=0
for dsf in "${DSF_FILES[@]}"; do
    basename_dsf="$(basename "${dsf}")"
    # Extract track number from filename
    if [[ "${basename_dsf}" =~ ^([0-9]+) ]]; then
        tn="${BASH_REMATCH[1]}"
    else
        continue
    fi

    # Use track name from txt file if available
    tn_num=$((10#${tn}))
    if [ -n "${TRACK_NAMES[${tn_num}]+x}" ]; then
        title="${TRACK_NAMES[${tn_num}]}"
    else
        # Fallback to filename parsing
        if [[ "${basename_dsf}" =~ ^[0-9]+\ *-\ *[^-]*-\ *(.+)\.dsf$ ]]; then
            title="${BASH_REMATCH[1]}"
        elif [[ "${basename_dsf}" =~ ^[0-9]+\ *[-_]\ *(.+)\.dsf$ ]]; then
            title="${BASH_REMATCH[1]}"
        else
            title="${basename_dsf%.dsf}"
            title="${title%.DSF}"
        fi
    fi

    # Clean title: remove CR and invalid chars
    title=$(echo "${title}" | tr -d '\r' | sed 's/[\/:<>"|?*]//g' | sed 's/  */ /g' | sed 's/^ //;s/ $//')
    [ -z "${title}" ] && title="Track ${tn}"

    out_file="${ALBUM_OUTPUT}/$(printf '%02d' "${tn_num}") ${title}.flac"
    [ -f "${out_file}" ] && { log "  Skipping (exists): ${title}"; continue; }

    log "  Track ${tn_num}/${total_tracks}: ${title}..."

    # Convert DSD to PCM FLAC with proper downsampling
    if ffmpeg -y -i "${dsf}" \
        -map 0:a:0 -vn -sn \
        -af "lowpass=f=${LOWPASS_FREQ},aresample=osr=${TARGET_RATE}" \
        -c:a flac -compression_level 8 \
        -sample_fmt s32 -bits_per_raw_sample "${BITS_PER_SAMPLE}" \
        -metadata "artist=${ARTIST}" \
        -metadata "album=${ALBUM_NAME}" \
        -metadata "track=${tn_num}/${total_tracks}" \
        -metadata "title=${title}" \
        "${out_file}" 2>>"${LOG_FILE}"; then
        log "    OK"
    else
        log "    ERROR (ffmpeg failed)"
        ((errors++))
    fi
done

# Copy images
image_count=0
while IFS= read -r -d '' img; do
    cp -p "${img}" "${ALBUM_OUTPUT}/"
    log "  Copied: $(basename "${img}")"
    ((image_count++))
done < <(find "${SOURCE_DIR}" -maxdepth 1 -type f \( -iname '*.jpg' -o -iname '*.jpeg' -o -iname '*.png' -o -iname '*.bmp' -o -iname '*.webp' -o -iname '*.tiff' -o -iname '*.tif' \) -print0)
[ "${image_count}" -eq 0 ] && log "  No images found"

log ""
log "============================================================"
log "Complete at $(date)"
log "Album: ${ALBUM_NAME}  Tracks: ${total_tracks}  Errors: ${errors}"
log "Output: ${ALBUM_OUTPUT}"
log "============================================================"
