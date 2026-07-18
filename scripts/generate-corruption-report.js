#!/usr/bin/env node
/**
 * Generate flac-corruption-report.html from a doctor scan text report.
 * Usage: node scripts/generate-corruption-report.js <report.txt> [output.html]
 */
const fs = require("fs");
const path = require("path");

const reportFile = process.argv[2] || "/Users/ihelio/code/soundrobe/reports/flac-metadata-report-2026-06-19T04-02-17-254Z.txt";
const outputFile = process.argv[3] || "/Users/ihelio/code/soundrobe/reports/flac-corruption-report.html";

const reportText = fs.readFileSync(reportFile, "utf8");
const lines = reportText.split("\n");
const jsonReport = (() => {
  try {
    return JSON.parse(reportText);
  } catch (_) {
    return null;
  }
})();

// ── Parse summary ───────────────────────────────────────────────────
const totalLine = lines.find(l => l.startsWith("Total files scanned"));
const okLine = lines.find(l => l.startsWith("  OK"));
const alertLine = lines.find(l => l.startsWith("  Broken"));
const warnLine = lines.find(l => l.startsWith("  Medium"));
const errorLine = lines.find(l => l.startsWith("  Errors"));
const vorbisLine = lines.find(l => l.startsWith("  Vorbis length mismatch"));
const audioLine = lines.find(l => l.startsWith("  Audio frame corrupted"));

const total = jsonReport?.summary?.total ?? parseInt(totalLine?.match(/\d+/)?.[0] || "0");
const ok = jsonReport?.summary?.clean ?? parseInt(okLine?.match(/\d+/)?.[0] || "0");
const alerts = jsonReport?.summary?.broken ?? parseInt(alertLine?.match(/\d+/)?.[0] || "0");
const minor = jsonReport?.summary?.minor ?? 0;
const warnings = jsonReport?.summary?.medium ?? parseInt(warnLine?.match(/\d+/)?.[0] || "0");
const errors = parseInt(errorLine?.match(/\d+/)?.[0] || "0");
const vorbisMismatch = parseInt(vorbisLine?.match(/\d+/)?.[0] || "0");
const audioCorrupted = parseInt(audioLine?.match(/\d+/)?.[0] || "0");

// ── Parse file entries ──────────────────────────────────────────────
const fileRegex = /^  (BROKEN_STRICT_DECODE_FAIL|ALERT_STRICT_DECODE_FAIL|MEDIUM_STRICT_DECODE_PLAYABLE|MEDIUM_STRICT_DECODE|WARNING_STRICT_DECODE(?:_PLAYABLE)?)\s+(.+?)\s+\(/;
const artists = {};
const mediumTypes = {
  missingMd5: { label: "Missing MD5", className: "medium-md5" },
  playableStrictDecode: { label: "Playable Strict Decode", className: "medium-playable" },
  md5Mismatch: { label: "MD5 Mismatch", className: "medium-md5-mismatch" },
  strictDecodeWarning: { label: "Other Strict Warning", className: "medium-other-warning" },
  otherMedium: { label: "Other Medium", className: "medium-other" },
};

function emptyMediumGroups() {
  return Object.fromEntries(Object.keys(mediumTypes).map(type => [type, []]));
}

function emptyMediumCounts() {
  return Object.fromEntries(Object.keys(mediumTypes).map(type => [type, 0]));
}

function ensureAlbum(artist, album) {
  if (!artists[artist]) artists[artist] = {};
  if (!artists[artist][album]) {
    artists[artist][album] = { fail: [], minor: [], warn: [], mediumGroups: emptyMediumGroups() };
  }
  return artists[artist][album];
}

function classifyMediumIssue(detail) {
  const issues = Array.isArray(detail?.issues) ? detail.issues : [];
  const message = String(detail?.strict?.message || "");
  if (message.includes("MD5 signature mismatch")) return "md5Mismatch";
  if (message.includes("cannot check MD5 signature since it was unset")) return "missingMd5";
  if (issues.includes("strict-decode-invalid-playable")) return "playableStrictDecode";
  if (issues.includes("strict-decode-warning")) return "strictDecodeWarning";
  return "otherMedium";
}

function addMediumFile(album, filename, type) {
  const mediumType = mediumTypes[type] ? type : "otherMedium";
  album.warn.push(filename);
  album.mediumGroups[mediumType].push(filename);
}

if (jsonReport?.details) {
  for (const [key, detail] of Object.entries(jsonReport.details)) {
    const bucket = detail.bucket;
    if (bucket === "clean") continue;

    const relativePath = detail.relativePath || key;
    const parts = relativePath.split("/");
    const artist = detail.artist || parts[0] || "Unknown Artist";
    const albumName = detail.album || parts.slice(1, -1).join(" / ") || path.dirname(relativePath);
    const filename = detail.track || path.basename(relativePath);
    const album = ensureAlbum(artist, albumName);

    if (bucket === "broken") {
      album.fail.push(filename);
    } else if (bucket === "minor") {
      album.minor.push(filename);
    } else if (bucket === "medium") {
      addMediumFile(album, filename, classifyMediumIssue(detail));
    }
  }
} else if (jsonReport?.diagnosis) {
  for (const [artist, albums] of Object.entries(jsonReport.diagnosis)) {
    for (const [album, tracks] of Object.entries(albums)) {
      for (const [filename, buckets] of Object.entries(tracks)) {
        const bucket = Array.isArray(buckets) ? buckets[0] : buckets;
        if (bucket === "clean") continue;
        const albumEntry = ensureAlbum(artist, album);
        if (bucket === "broken") {
          albumEntry.fail.push(filename);
        } else if (bucket === "minor") {
          albumEntry.minor.push(filename);
        } else {
          addMediumFile(albumEntry, filename, "otherMedium");
        }
      }
    }
  }
} else {
  for (const line of lines) {
    const m = line.match(fileRegex);
    if (!m) continue;
    const severity = m[1];
    const filepath = m[2];

    const parts = filepath.split("/");
    if (parts.length < 2) continue;

    const artist = parts[0];
    const album = parts.slice(1, -1).join(" / ") || parts[1];
    const filename = parts[parts.length - 1];

    const albumEntry = ensureAlbum(artist, album);

    if (severity.startsWith("BROKEN") || severity.startsWith("ALERT")) {
      albumEntry.fail.push(filename);
    } else if (severity.includes("PLAYABLE")) {
      addMediumFile(albumEntry, filename, "playableStrictDecode");
    } else {
      addMediumFile(albumEntry, filename, "strictDecodeWarning");
    }
  }
}

const sortedArtists = Object.entries(artists)
  .map(([name, albums]) => {
    let fail = 0, minor = 0, warn = 0;
    const mediumCounts = emptyMediumCounts();
    for (const a of Object.values(albums)) {
      fail += a.fail.length;
      minor += a.minor.length;
      warn += a.warn.length;
      for (const type of Object.keys(mediumTypes)) {
        mediumCounts[type] += a.mediumGroups?.[type]?.length || 0;
      }
    }
    return { name, albums, fail, minor, warn, mediumCounts };
  })
  .sort((a, b) => {
    if ((a.fail > 0) !== (b.fail > 0)) return a.fail > 0 ? -1 : 1;
    return (b.fail + b.warn) - (a.fail + a.warn);
  });

const mediumTotals = sortedArtists.reduce((totals, artist) => {
  for (const type of Object.keys(mediumTypes)) {
    totals[type] += artist.mediumCounts[type] || 0;
  }
  return totals;
}, emptyMediumCounts());
const visibleMediumTypes = Object.entries(mediumTotals).filter(([, count]) => count > 0);

// ── Detect report timestamp from filename ──────────────────────────
const tsMatch = reportFile.match(/flac-metadata-report-(\d{4})-(\d{2})-(\d{2})T(\d{2})-(\d{2})-(\d{2})/);
const reportDate = tsMatch
  ? `${tsMatch[1]}-${tsMatch[2]}-${tsMatch[3]} ${tsMatch[4]}:${tsMatch[5]}:${tsMatch[6]}`
  : new Date().toLocaleString("zh-CN");

// ── Generate HTML ───────────────────────────────────────────────────
function esc(s) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

function plural(count, singular, pluralText) {
  return `${count} ${count === 1 ? singular : pluralText}`;
}

let html = `<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>FLAC Audio Corruption Report</title>
<style>
  *{margin:0;padding:0;box-sizing:border-box}
  body{font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif;background:#0d1117;color:#c9d1d9;padding:20px}
  h1{font-size:24px;margin-bottom:4px}
  .subtitle{color:#8b949e;font-size:13px;margin-bottom:20px}
  .summary{display:flex;gap:12px;margin-bottom:20px;flex-wrap:wrap}
  .stat-card{background:#161b22;border:1px solid #30363d;border-radius:8px;padding:14px 18px;min-width:120px}
  .stat-card .num{font-size:26px;font-weight:600}
  .stat-card .label{font-size:11px;color:#8b949e;text-transform:uppercase;letter-spacing:.4px;margin-top:2px}
  .stat-card.danger .num{color:#f85149}
  .stat-card.danger{border-color:#f85149}
  .stat-card.warn .num{color:#d29922}
  .stat-card.warn{border-color:#d29922}
  .stat-card.minor .num{color:#58a6ff}
  .stat-card.minor{border-color:#58a6ff}
  .stat-card.ok .num{color:#3fb950}
  .stat-card.ok{border-color:#3fb950}
  .stat-card.info .num{color:#58a6ff}
  .stat-card.info{border-color:#58a6ff}
  .medium-breakdown{background:#161b22;border:1px solid #30363d;border-radius:8px;margin-bottom:18px;padding:12px 14px}
  .medium-breakdown h2{font-size:13px;margin-bottom:10px;color:#e6edf3}
  .medium-grid{display:flex;gap:10px;flex-wrap:wrap}
  .medium-chip{border:1px solid #30363d;border-radius:6px;padding:8px 10px;min-width:150px;background:#0d1117}
  .medium-chip .num{font-size:18px;font-weight:600;color:#d29922}
  .medium-chip .label{font-size:11px;color:#8b949e;text-transform:uppercase;letter-spacing:.3px}
  .search-box{background:#0d1117;border:1px solid #30363d;border-radius:6px;padding:10px 14px;color:#c9d1d9;font-size:14px;width:100%;margin-bottom:16px;outline:none}
  .search-box:focus{border-color:#58a6ff}
  .artist{background:#161b22;border:1px solid #30363d;border-radius:8px;margin-bottom:10px;overflow:hidden}
  .artist-header{display:flex;justify-content:space-between;align-items:center;padding:10px 14px;cursor:pointer;user-select:none}
  .artist-header:hover{background:#1c2128}
  .artist-name{font-size:15px;font-weight:600}
  .artist-stats{display:flex;gap:10px;font-size:12px;align-items:center}
  .artist-stats .alert{color:#f85149}
  .artist-stats .warn{color:#d29922}
  .artist-stats .clean{color:#3fb950}
  .artist-stats .minor{color:#58a6ff}
  .toggle-icon{color:#8b949e;font-size:10px;transition:transform .2s}
  .toggle-icon.open{transform:rotate(90deg)}
  .artist-body{display:none;border-top:1px solid #30363d}
  .artist-body.open{display:block}
  .album{padding:8px 14px 8px 28px}
  .album+.album{border-top:1px solid #21262d}
  .album-header{display:flex;justify-content:space-between;align-items:center;margin-bottom:4px}
  .album-name{font-size:13px;font-weight:500;color:#e6edf3}
  .album-stats{font-size:11px}
  .album-stats .alert{color:#f85149}
  .album-stats .warn{color:#d29922}
  .file-list{padding-left:14px;font-size:11px;color:#8b949e;line-height:1.6;max-height:200px;overflow-y:auto}
  .file-list .alert{color:#f85149}
  .file-list .warn{color:#d29922}
  .file-list .minor{color:#58a6ff}
  .file-group-title{font-size:11px;color:#e3b341;margin-top:6px;margin-bottom:2px;font-weight:600}
  .file-group-title.medium-md5{color:#d29922}
  .file-group-title.medium-playable{color:#db6d28}
  .file-group-title.medium-md5-mismatch{color:#f0883e}
  .file-group-title.medium-other-warning{color:#e3b341}
  .file-group-title.medium-other{color:#a371f7}
  .tag{display:inline-block;font-size:10px;padding:1px 7px;border-radius:10px;margin-left:6px;font-weight:500}
  .tag-alert{background:#f8514920;color:#f85149;border:1px solid #f8514940}
  .tag-warn{background:#d2992220;color:#d29922;border:1px solid #d2992240}
  .tag-mixed{background:#db6d2820;color:#db6d28;border:1px solid #db6d2840}
  .tag-minor{background:#58a6ff20;color:#58a6ff;border:1px solid #58a6ff40}
  .expand-all{font-size:12px;color:#58a6ff;cursor:pointer;margin-bottom:12px;display:inline-block}
  .expand-all:hover{text-decoration:underline}
</style>
</head>
<body>
<h1>🎵 FLAC Audio Corruption Report</h1>
<p class="subtitle">Scan: <code>${esc(reportDate)}</code> &mdash; ${total} files scanned across ${sortedArtists.length} affected artists</p>

<div class="summary">
  <div class="stat-card ok">
    <div class="num">${ok}</div>
    <div class="label">Clean Files</div>
  </div>
  <div class="stat-card danger">
    <div class="num">${alerts}</div>
    <div class="label">Broken Files</div>
  </div>
  <div class="stat-card minor">
    <div class="num">${minor}</div>
    <div class="label">Minor Files (fixable)</div>
  </div>
  <div class="stat-card warn">
    <div class="num">${warnings}</div>
    <div class="label">Medium Files</div>
  </div>
  <div class="stat-card info">
    <div class="num">${sortedArtists.length}</div>
    <div class="label">Affected Artists</div>
  </div>
</div>

${visibleMediumTypes.length > 0 ? `
<section class="medium-breakdown">
  <h2>Medium Issue Types</h2>
  <div class="medium-grid">
    ${visibleMediumTypes.map(([type, count]) => `
    <div class="medium-chip">
      <div class="num">${count}</div>
      <div class="label">${esc(mediumTypes[type].label)}</div>
    </div>`).join("")}
  </div>
</section>` : ""}

<input class="search-box" type="text" placeholder="Search artist name..." oninput="filterArtists(this.value)">
<div style="margin-bottom:12px"><span class="expand-all" onclick="toggleAll()">${'[Expand All / Collapse All]'}</span></div>

<div id="artist-list">`;

for (const artist of sortedArtists) {
  const { name, albums, fail, minor: minorCount, warn } = artist;
  const hasFail = fail > 0;
  const hasMinor = minorCount > 0;
  const hasWarn = warn > 0;

  let badgeClass = "tag-clean";
  let badgeLabel = "";
  if (hasFail && hasWarn) {
    badgeClass = "tag-mixed";
    badgeLabel = `${plural(fail, "broken", "broken")} + ${plural(warn, "medium", "medium")}`;
  } else if (hasFail) {
    badgeClass = "tag-alert";
    badgeLabel = plural(fail, "broken", "broken");
  } else if (hasMinor && hasWarn) {
    badgeClass = "tag-mixed";
    badgeLabel = `${plural(minorCount, "minor", "minor")} + ${plural(warn, "medium", "medium")}`;
  } else if (hasMinor) {
    badgeClass = "tag-minor";
    badgeLabel = plural(minorCount, "minor", "minor");
  } else {
    badgeClass = "tag-warn";
    badgeLabel = plural(warn, "medium", "medium");
  }

  const artistStatsParts = [];
  if (hasFail) artistStatsParts.push(`<span class="alert">\u26A0 ${fail}</span>`);
  if (hasMinor) artistStatsParts.push(`<span class="minor">\u26A0 ${minorCount}</span>`);
  if (hasWarn) artistStatsParts.push(`<span class="warn">\u26A0 ${warn}</span>`);

  html += `
<div class="artist" data-name="${esc(name.toLowerCase())}">
  <div class="artist-header" onclick="toggle(this)">
    <span class="artist-name">${esc(name)} <span class="tag ${badgeClass}">${esc(badgeLabel)}</span></span>
    <span class="artist-stats">
      ${artistStatsParts.join(" ")}
      <span class="toggle-icon">&#9654;</span>
    </span>
  </div>
  <div class="artist-body">`;

  // Sort albums: broken first, then by total issues.
  const sortedAlbums = Object.entries(albums)
    .filter(([, a]) => a.fail.length > 0 || a.minor.length > 0 || a.warn.length > 0)
    .sort((a, b) => (b[1].fail.length + b[1].minor.length + b[1].warn.length) - (a[1].fail.length + a[1].minor.length + a[1].warn.length));

  for (const [albumName, album] of sortedAlbums) {
    const fCount = album.fail.length;
    const mCount = album.minor.length;
    const wCount = album.warn.length;
    const albumStatsParts = [];
    if (fCount > 0) albumStatsParts.push(`<span class="alert">\u26A0 ${fCount}</span>`);
    if (mCount > 0) albumStatsParts.push(`<span class="minor">\u26A0 ${mCount}</span>`);
    if (wCount > 0) albumStatsParts.push(`<span class="warn">\u26A0 ${wCount}</span>`);
    html += `
    <div class="album">
      <div class="album-header">
        <span class="album-name">${esc(albumName)}</span>
        <span class="album-stats">
          ${albumStatsParts.join(" ")}
        </span>
      </div>`;

    if (fCount > 0) {
      html += `<div class="file-list">`;
      for (const f of album.fail) {
        html += `<div class="alert">&#9888; ${esc(f)}</div>`;
      }
      html += `</div>`;
    }
    if (mCount > 0) {
      html += `<div class="file-list">`;
      for (const f of album.minor) {
        html += `<div class="minor">&#9888; ${esc(f)}</div>`;
      }
      html += `</div>`;
    }
    if (wCount > 0) {
      html += `<div class="file-list">`;
      for (const [type, config] of Object.entries(mediumTypes)) {
        const files = album.mediumGroups?.[type] || [];
        if (files.length === 0) continue;
        html += `<div class="file-group-title ${config.className}">${esc(config.label)} (${files.length})</div>`;
        for (const f of files) {
          html += `<div class="warn">&#9888; ${esc(f)}</div>`;
        }
      }
      html += `</div>`;
    }

    html += `</div>`;
  }

  html += `
  </div>
</div>`;
}

html += `
</div>

<script>
function toggle(el) {
  var body = el.nextElementSibling;
  var icon = el.querySelector(".toggle-icon");
  body.classList.toggle("open");
  icon.classList.toggle("open");
}
function filterArtists(val) {
  var q = val.toLowerCase().trim();
  document.querySelectorAll(".artist").forEach(function(el) {
    el.style.display = !q || el.getAttribute("data-name").includes(q) ? "" : "none";
  });
}
function toggleAll() {
  var open = document.querySelectorAll(".artist-body.open").length === 0;
  document.querySelectorAll(".artist-body").forEach(function(b) { b.classList.toggle("open", open); });
  document.querySelectorAll(".toggle-icon").forEach(function(i) { i.classList.toggle("open", open); });
}
</script>
</body>
</html>`;

fs.writeFileSync(outputFile, html, "utf8");
const failedTotal = sortedArtists.reduce((s, a) => s + a.fail, 0);
const minorTotal = sortedArtists.reduce((s, a) => s + a.minor, 0);
const warnedTotal = sortedArtists.reduce((s, a) => s + a.warn, 0);
console.log("Report: " + outputFile);
console.log("Artists: " + sortedArtists.length + " | Broken: " + failedTotal + " | Minor: " + minorTotal + " | Medium: " + warnedTotal);
console.log("Size: " + (html.length / 1024).toFixed(0) + " KB");
