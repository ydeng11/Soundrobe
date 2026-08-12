export type ConvertDirection =
  | "filename-to-tags"
  | "tag-to-tags"
  | "tags-to-filename";

export type ConvertSourceTag =
  | "title"
  | "artist"
  | "album"
  | "year"
  | "genre"
  | "albumArtist"
  | "composer"
  | "comment"
  | "track"
  | "disc";

export interface ConvertFieldMap {
  title?: string;
  artist?: string;
  album?: string;
  year?: string;
  genre?: string;
  albumArtist?: string;
  composer?: string;
  comment?: string;
  track?: string;
  disc?: string;
}

export interface ConvertTrackData {
  filename: string;
  title: string | null;
  artist: string | null;
  album: string | null;
  year: string | null;
  track: number | null;
  genre: string | null;
  albumArtist: string | null;
  composer: string | null;
  comment: string | null;
  discNumber: number | null;
}

export interface ConvertPreset {
  label: string;
  direction: ConvertDirection;
  pattern: string;
  description: string;
  sourceTag?: ConvertSourceTag;
}

export const CONVERT_FIELD_LABELS: Record<string, string> = {
  title: "Title",
  artist: "Artist",
  album: "Album",
  year: "Year",
  track: "Track",
  genre: "Genre",
  albumArtist: "Album Artist",
  composer: "Composer",
  comment: "Comment",
  disc: "Disc",
};

export const CONVERT_SOURCE_TAGS: Array<{
  key: ConvertSourceTag;
  label: string;
}> = [
  { key: "title", label: "Title" },
  { key: "artist", label: "Artist" },
  { key: "album", label: "Album" },
  { key: "year", label: "Year" },
  { key: "genre", label: "Genre" },
  { key: "albumArtist", label: "Album Artist" },
  { key: "composer", label: "Composer" },
  { key: "comment", label: "Comment" },
  { key: "track", label: "Track" },
  { key: "disc", label: "Disc" },
];

export const DEFAULT_CONVERT_PRESETS: ConvertPreset[] = [
  {
    label: "Track Title",
    direction: "filename-to-tags",
    pattern: "%{track}% %{title}%",
    description: "01 Song Title -> track=01, title=Song Title",
  },
  {
    label: "Track - Title",
    direction: "filename-to-tags",
    pattern: "%{track}% - %{title}%",
    description: "01 - Song Title -> track=01, title=Song Title",
  },
  {
    label: "Artist - Title",
    direction: "filename-to-tags",
    pattern: "%{artist}% - %{title}%",
    description: "Artist - Song Title -> artist=Artist, title=Song Title",
  },
  {
    label: "Track - Artist - Title",
    direction: "filename-to-tags",
    pattern: "%{track}% - %{artist}% - %{title}%",
    description: "2 - ABC - XYZ -> track=2, artist=ABC, title=XYZ",
  },
  {
    label: "Title has Artist - Title",
    direction: "tag-to-tags",
    sourceTag: "title",
    pattern: "%{artist}% - %{title}%",
    description: "Title tag: Artist - Song Title -> artist/title tags",
  },
  {
    label: "Title has Track - Title",
    direction: "tag-to-tags",
    sourceTag: "title",
    pattern: "%{track}% - %{title}%",
    description: "Title tag: 01 - Song Title -> track/title tags",
  },
  {
    label: "Artist has Album Artist - Artist",
    direction: "tag-to-tags",
    sourceTag: "artist",
    pattern: "%{albumArtist}% - %{artist}%",
    description: "Artist tag: Album Artist - Artist -> album artist/artist tags",
  },
  {
    label: "Track - Title filename",
    direction: "tags-to-filename",
    pattern: "%{track}% - %{title}%%{ext}%",
    description: "Build filename from tags: %{track}% - %{title}%%{ext}%",
  },
  {
    label: "Artist - Title filename",
    direction: "tags-to-filename",
    pattern: "%{artist}% - %{title}%%{ext}%",
    description: "Build filename from tags: %{artist}% - %{title}%%{ext}%",
  },
];

const VALID_PARSE_FIELDS = new Set([
  "title",
  "artist",
  "album",
  "year",
  "track",
  "genre",
  "albumArtist",
  "composer",
  "comment",
  "disc",
  "ext",
  "filename",
]);

function padPosition(n: number | null): string {
  if (n == null) return "01";
  return String(n).padStart(2, "0");
}

function escapeLiteral(text: string): string {
  return text.replace(/[.+*?^${}()|[\]\\-]/g, "\\$&");
}

function tokenizePattern(pattern: string): Array<{
  type: "literal" | "field";
  value: string;
}> {
  const tokens: Array<{ type: "literal" | "field"; value: string }> = [];
  const normalizedPattern = pattern.replace(/}%\{/g, "}%%{");
  const fieldRegex = /%\{([A-Za-z][A-Za-z0-9]*)\}%/g;
  let lastIndex = 0;
  let match: RegExpExecArray | null;

  while ((match = fieldRegex.exec(normalizedPattern)) !== null) {
    if (match.index > lastIndex) {
      tokens.push({
        type: "literal",
        value: normalizedPattern.slice(lastIndex, match.index),
      });
    }
    tokens.push({ type: "field", value: match[1] });
    lastIndex = match.index + match[0].length;
  }

  if (lastIndex < normalizedPattern.length) {
    tokens.push({ type: "literal", value: normalizedPattern.slice(lastIndex) });
  }

  return tokens;
}

function patternToRegex(pattern: string): RegExp {
  const tokens = tokenizePattern(pattern);
  const fields = tokens.filter((t) => t.type === "field");
  if (fields.length === 0) {
    throw new Error("Pattern must include at least one placeholder like %{title}%");
  }

  const seen = new Set<string>();
  const parts: string[] = ["^"];
  let fieldIndex = 0;

  for (const token of tokens) {
    if (token.type === "literal") {
      for (const segment of token.value.split(/(\s+)/)) {
        if (/^\s+$/.test(segment)) {
          parts.push("\\s+");
        } else if (segment.length > 0) {
          parts.push(escapeLiteral(segment));
        }
      }
      continue;
    }

    const name = token.value;
    if (!VALID_PARSE_FIELDS.has(name)) {
      throw new Error(`Unsupported placeholder: %{${name}}%`);
    }
    if (seen.has(name)) {
      throw new Error(`Placeholder can only be used once: %{${name}}%`);
    }
    seen.add(name);

    const isLastField = fieldIndex === fields.length - 1;
    if (name === "track" || name === "disc") {
      parts.push(`(?<${name}>\\d+)`);
    } else if (name === "year") {
      parts.push("(?<year>\\d{4})");
    } else if (name === "ext") {
      parts.push("(?<ext>\\.[^.]+)");
    } else if (name === "filename") {
      parts.push("(?<filename>.+)");
    } else {
      parts.push(`(?<${name}>.${isLastField ? "+" : "+?"})`);
    }
    fieldIndex++;
  }

  parts.push("$");
  return new RegExp(parts.join(""));
}

export function parseTextWithConvertPattern(
  pattern: string,
  text: string,
): { fields: ConvertFieldMap; displayResult: string } | { error: string } {
  try {
    const regex = patternToRegex(pattern);
    const match = text.match(regex);
    if (!match) {
      return { error: "No match - pattern does not fit this value" };
    }

    const fields: ConvertFieldMap = {};
    for (const [name, value] of Object.entries(match.groups ?? {})) {
      if (
        value !== undefined &&
        value.trim() &&
        name !== "ext" &&
        name !== "filename"
      ) {
        fields[name as keyof ConvertFieldMap] = value.trim();
      }
    }

    const displayResult = Object.entries(fields)
      .map(([key, value]) => `${CONVERT_FIELD_LABELS[key] ?? key}=${value}`)
      .join(", ");

    return {
      fields,
      displayResult: displayResult || "(no writable tags)",
    };
  } catch (error) {
    return { error: `Invalid pattern: ${(error as Error).message}` };
  }
}

export function parseFilenameWithConvertPattern(
  pattern: string,
  filename: string,
): { fields: ConvertFieldMap; displayResult: string } | { error: string } {
  const text = pattern.includes("%{ext}%")
    ? filename
    : filename.replace(/\.[^.]+$/, "");
  return parseTextWithConvertPattern(pattern, text);
}

export function buildFilenameFromConvertPattern(
  pattern: string,
  data: ConvertTrackData | null,
): string {
  if (!data) return pattern;
  const normalizedPattern = pattern.replace(/}%\{/g, "}%%{");
  const ext = data.filename.includes(".")
    ? "." + data.filename.split(".").pop()
    : ".mp3";
  const values: Record<string, string> = {
    title: data.title ?? "Unknown",
    artist: data.artist ?? "Unknown Artist",
    album: data.album ?? "Unknown Album",
    year: data.year ?? "0000",
    track: padPosition(data.track),
    genre: data.genre ?? "Unknown",
    albumArtist: data.albumArtist ?? "Unknown Album Artist",
    composer: data.composer ?? "Unknown Composer",
    comment: data.comment ?? "",
    disc: padPosition(data.discNumber),
    ext,
    filename: data.filename.replace(/\.[^.]+$/, ""),
  };

  return normalizedPattern.replace(
    /%\{([A-Za-z][A-Za-z0-9]*)\}%/g,
    (_, name: string) => {
      return values[name] ?? "";
    },
  );
}

export function getConvertSourceValue(
  data: ConvertTrackData,
  sourceTag: ConvertSourceTag,
): string {
  switch (sourceTag) {
    case "track":
      return data.track == null ? "" : String(data.track);
    case "disc":
      return data.discNumber == null ? "" : String(data.discNumber);
    default:
      return data[sourceTag] ?? "";
  }
}
