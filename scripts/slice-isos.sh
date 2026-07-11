#!/usr/bin/env bash
# ============================================================================
# slice-isos.sh
#
# Slices audio ISO images into properly-named FLAC tracks.
# Tested with Teresa Teng (邓丽君) K2HD SACD and raw CD audio images.
#
# Supported ISO formats:
#
# Format 1 — K2HD SACD (mountable UDF ISO):
#   Contains 2C_AUDIO/ with TRACK*.2CH files.
#   Audio: 24-bit 96kHz stereo PCM, sector-packed.
#   Each 2048-byte sector: 32-byte header + 2016 bytes s24le audio.
#   Each TRACK file: 2048-byte file header + data sectors.
#
# Format 2 — Raw CD Audio (raw 16-bit 44100Hz PCM, no filesystem):
#   No track markers; splits equally by duration.
#
# Output: ~/Music/<Artist>/<Album>/NN Title.flac
#
# Requires: hdiutil, ffmpeg, python3
# ============================================================================

set -euo pipefail

SOURCE_DIR="${SOURCE_DIR:-/Volumes/downloads/邓丽君}"
ARTIST="${ARTIST:-Teresa Teng}"
OUTPUT_BASE="${OUTPUT_BASE:-${HOME}/Music/${ARTIST}}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
LOG_FILE="${SCRIPT_DIR}/slice-isos.log"

mkdir -p "${OUTPUT_BASE}"

log() { echo "$@" | tee -a "${LOG_FILE}"; }

log "============================================================"
log "Slice ISOs started at $(date)"
log "Source: ${SOURCE_DIR}"
log "Output: ${OUTPUT_BASE}"
log "============================================================"

TOTAL_ALBUMS=0
TOTAL_TRACKS=0
TOTAL_ERRORS=0

# Use functions for safe incrementing (bash 3.2 compat + set -e)
inc_albums()  { TOTAL_ALBUMS=$((TOTAL_ALBUMS + 1)); }
inc_tracks()  { TOTAL_TRACKS=$((TOTAL_TRACKS + 1)); }
inc_errors()  { TOTAL_ERRORS=$((TOTAL_ERRORS + 1)); }

# ---------------------------------------------------------------
# Parse track titles from 专辑曲目.txt
# Uses global array TITLES (indexed by track number)
# ---------------------------------------------------------------
parse_titles() {
    local file="$1"
    local line
    TITLES=()
    if [ ! -f "${file}" ]; then return 0; fi
    while IFS= read -r line; do
        line=$(echo "${line}" | sed 's/^\xEF\xBB\xBF//' | sed 's/^[[:space:]]*//' | sed 's/[[:space:]]*$//')
        [ -z "${line}" ] && continue
        if [[ "${line}" =~ ^([0-9]+)[.、\)][[:space:]]*(.+)$ ]]; then
            local tn=$((10#${BASH_REMATCH[1]}))
            local title="${BASH_REMATCH[2]}"
            title=$(echo "${title}" | sed 's/[（(][^）)]*[）)]//g' | sed 's/^[[:space:]]*//' | sed 's/[[:space:]]*$//')
            TITLES[${tn}]="${title}"
        fi
    done < "${file}"
}

# ---------------------------------------------------------------
# Copy image files (cover art) from source album dir to output
# ---------------------------------------------------------------
copy_images() {
    local src="$1" dst="$2"
    local count=0
    while IFS= read -r -d '' img; do
        cp -p "${img}" "${dst}/"
        log "  Copied: $(basename "${img}")"
        ((count++))
    done < <(find "${src}" -maxdepth 1 -type f \( -iname '*.jpg' -o -iname '*.jpeg' -o -iname '*.png' -o -iname '*.bmp' -o -iname '*.webp' -o -iname '*.tiff' -o -iname '*.tif' \) -print0)
    [ "${count}" -eq 0 ] && log "  No images found in ${src}" || true
}

# ---------------------------------------------------------------
# Extract K2HD track from TRACK*.2CH → stdout (raw s24le audio)
# ---------------------------------------------------------------
extract_k2hd() {
    python3 -c "
import sys
with open('$1', 'rb') as f:
    f.read(2048)                    # file header
    while True:
        s = f.read(2048)
        if len(s) < 2048: break
        sys.stdout.buffer.write(s[32:])  # strip sector header
"
}

# ---------------------------------------------------------------
# Process a mountable K2HD UDF ISO
# Optional $5: pre-extracted audio directory (skips mounting)
# ---------------------------------------------------------------
process_k2hd() {
    local iso="$1" album_dir="$2" album_name="$3" output="$4" pre_audio_dir="${5:-}"
    local mount_point audio_dir track_count track_file title safe_title out_file

    if [ -n "${pre_audio_dir}" ] && [ -d "${pre_audio_dir}" ]; then
        audio_dir="${pre_audio_dir}"
        log "  Using extracted audio dir: ${audio_dir}"
    else
        mount_point=$(hdiutil attach -readonly "${iso}" 2>/dev/null | tail -1 | awk '{print $NF}' || true)
        [ -z "${mount_point}" ] && { log "  ERROR: cannot mount ${iso}"; return 1; }
        log "  Mounted at: ${mount_point}"

        audio_dir="${mount_point}/2C_AUDIO"
        [ ! -d "${audio_dir}" ] && audio_dir=$(find "${mount_point}" -type d -name "2C_AUDIO" 2>/dev/null | head -1)
        if [ -z "${audio_dir}" ]; then
            log "  ERROR: no 2C_AUDIO directory"
            hdiutil detach "${mount_point}" 2>/dev/null
            return 1
        fi
    fi

    parse_titles "${album_dir}/专辑曲目.txt"

    # Count tracks
    track_count=0
    for f in "${audio_dir}"/TRACK*.2CH; do
        [ -f "$f" ] && ((track_count++))
    done
    log "  Tracks: ${track_count}"

    local tn=0
    for track_file in "${audio_dir}"/TRACK*.2CH; do
        [ -f "${track_file}" ] || continue
        tn=$((tn + 1))
        title="${TITLES[${tn}]:-Track ${tn}}"
        safe_title=$(echo "${title}" | sed 's/[\/:<>"|?*]//g' | sed 's/  */ /g' | sed 's/^ //;s/ $//')
        [ -z "${safe_title}" ] && safe_title="Track${tn}"
        out_file="${output}/$(printf '%02d' ${tn}) ${safe_title}.flac"

        log "  Track ${tn}: ${title}..."
        extract_k2hd "${track_file}" | ffmpeg -y -f s24le -ar 96000 -ac 2 \
            -i pipe:0 -compression_level 8 \
            -metadata "artist=${ARTIST}" \
            -metadata "album=${album_name}" \
            -metadata "track=${tn}/${track_count}" \
            -metadata "title=${title}" \
            "${out_file}" 2>>"${LOG_FILE}"

        if [ $? -eq 0 ] && [ -f "${out_file}" ]; then
            log "    OK"
            inc_tracks
        else
            log "    ERROR"
            inc_errors
        fi
    done

    if [ -n "${mount_point:-}" ]; then
        hdiutil detach "${mount_point}" 2>/dev/null || true
    fi
}

# ---------------------------------------------------------------
# Process a raw CD audio ISO (16-bit 44100Hz PCM, no filesystem)
# ---------------------------------------------------------------
process_raw() {
    local iso="$1" album_dir="$2" album_name="$3" output="$4"
    local track_list="$5" num_tracks i

    # Parse track titles
    parse_titles "${track_list}"
    num_tracks=${#TITLES[@]}
    [ "${num_tracks}" -eq 0 ] && num_tracks=20
    log "  Tracks: ${num_tracks}"

    # Read full ISO as raw 16-bit 44100Hz PCM into a temp WAV
    local wav_tmp="${output}/_temp.wav"
    log "  Converting ISO to WAV..."
    ffmpeg -y -f s16le -ar 44100 -ac 2 -i "${iso}" "${wav_tmp}" 2>>"${LOG_FILE}"

    # Get duration
    local duration
    duration=$(ffprobe -v error -show_entries format=duration \
        -of default=noprint_wrappers=1:nokey=1 "${wav_tmp}" 2>/dev/null)
    local seg_dur
    seg_dur=$(python3 -c "print(${duration} / ${num_tracks})")
    log "  Duration: ${duration%.*}s — ${seg_dur%.*}s per track"

    # Split into equal segments using ffmpeg segment muxer
    local temp_pattern="${output}/%02d_temp.wav"
    ffmpeg -y -i "${wav_tmp}" -f segment -segment_time "${seg_dur}" \
        -c copy -reset_timestamps 1 "${temp_pattern}" 2>>"${LOG_FILE}"

    rm -f "${wav_tmp}"

    # Convert each segment to FLAC with proper metadata
    for temp_f in "${output}"/*_temp.wav; do
        [ -f "${temp_f}" ] || continue
        local base tn title safe_title
        base=$(basename "${temp_f}" _temp.wav)
        tn=$((10#${base} + 1))
        [ ${tn} -gt ${num_tracks} ] && { rm -f "${temp_f}"; continue; }
        title="${TITLES[${tn}]:-Track ${tn}}"
        safe_title=$(echo "${title}" | sed 's/[\/:<>"|?*]//g' | sed 's/  */ /g' | sed 's/^ //;s/ $//')
        [ -z "${safe_title}" ] && safe_title="Track${tn}"
        out_file="${output}/$(printf '%02d' ${tn}) ${safe_title}.flac"

        log "  Track ${tn}: ${title}..."
        ffmpeg -y -i "${temp_f}" -compression_level 8 \
            -metadata "artist=${ARTIST}" \
            -metadata "album=${album_name}" \
            -metadata "track=${tn}/${num_tracks}" \
            -metadata "title=${title}" \
            "${out_file}" 2>>"${LOG_FILE}"

        rm -f "${temp_f}"
        if [ -f "${out_file}" ]; then
            log "    OK"
            inc_tracks
        else
            log "    ERROR"
            inc_errors
        fi
    done
}

# ===============================================================
# Main
# ===============================================================

while IFS= read -r -d '' iso_file; do

    album_dir="$(dirname "${iso_file}")"
    album_name="$(basename "${album_dir}")"
    album_output="${OUTPUT_BASE}/${album_name}"

    log ""
    log "--- ${album_name} ---"
    mkdir -p "${album_output}"

    # Try mounting as K2HD (UDF) ISO
    mount_point=$(hdiutil attach -readonly "${iso_file}" 2>/dev/null | tail -1 | awk '{print $NF}' || true)
    if [ -n "${mount_point}" ] && [ -d "${mount_point}" ]; then
        audio_dir="${mount_point}/2C_AUDIO"
        [ ! -d "${audio_dir}" ] && audio_dir=$(find "${mount_point}" -type d -name "2C_AUDIO" 2>/dev/null | head -1)
        if [ -n "${audio_dir}" ]; then
            process_k2hd "${iso_file}" "${album_dir}" "${album_name}" "${album_output}"
            copy_images "${album_dir}" "${album_output}"
            inc_albums
            continue
        fi
        hdiutil detach "${mount_point}" 2>/dev/null
    fi

    # Fallback: extract with 7z if it contains 2C_AUDIO
    log "  Checking for 2C_AUDIO/TRACK in ISO..."
    _7z_tmp=$(mktemp)
    if 7z l "${iso_file}" 2>&1 > "${_7z_tmp}" && grep -q '2C_AUDIO/TRACK' "${_7z_tmp}"; then
        rm -f "${_7z_tmp}"
        log "  Found 2C_AUDIO/TRACK - extracting with 7z"
        extract_dir=$(mktemp -d)
        log "  Extracting TRACK files with 7z..."
        7z x -o"${extract_dir}" "${iso_file}" '2C_AUDIO/TRACK*.2CH' >/dev/null 2>&1
        audio_dir="${extract_dir}/2C_AUDIO"
        if [ -d "${audio_dir}" ] && ls "${audio_dir}"/TRACK*.2CH >/dev/null 2>&1; then
            process_k2hd "${iso_file}" "${album_dir}" "${album_name}" "${album_output}" "${audio_dir}"
            rm -rf "${extract_dir}"
            copy_images "${album_dir}" "${album_output}"
            inc_albums
            continue
        fi
        rm -rf "${extract_dir}"
    else
        rm -f "${_7z_tmp}"
        log "  2C_AUDIO/TRACK not found - trying raw CD audio"
    fi

    # Fallback: raw CD audio (only if track list exists)
    if [ -f "${album_dir}/专辑曲目.txt" ]; then
        process_raw "${iso_file}" "${album_dir}" "${album_name}" "${album_output}" "${album_dir}/专辑曲目.txt"
        copy_images "${album_dir}" "${album_output}"
        inc_albums
    else
        log "  SKIP: no track list and not a standard K2HD ISO"
    fi
done < <(find "${SOURCE_DIR}" -maxdepth 2 -name "*.iso" -print0 | sort -zV)

log ""
log "============================================================"
log "Complete at $(date)"
log "Albums: ${TOTAL_ALBUMS}  Tracks: ${TOTAL_TRACKS}  Errors: ${TOTAL_ERRORS}"
log "Output: ${OUTPUT_BASE}"
log "============================================================"
