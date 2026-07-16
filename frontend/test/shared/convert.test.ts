import { describe, expect, it } from "vitest";
import {
  buildFilenameFromConvertPattern,
  getConvertSourceValue,
  parseFilenameWithConvertPattern,
  parseTextWithConvertPattern,
  type ConvertTrackData,
} from "../../src/shared/convert";

const track: ConvertTrackData = {
  filename: "01 - Existing Title.flac",
  title: "Artist Name - Song Title",
  artist: "Existing Artist",
  album: "Existing Album",
  year: "2026",
  track: 1,
  genre: "Pop",
  albumArtist: "Existing Album Artist",
  composer: "Existing Composer",
  comment: "Existing Comment",
  discNumber: 2,
};

describe("ConvertService", () => {
  it("extracts filename tags from placeholder positions", () => {
    const result = parseFilenameWithConvertPattern(
      "%{track}% - %{artist}% - %{title}%",
      "02 - A-Lin - Best Song.flac",
    );

    expect(result).toEqual({
      fields: {
        track: "02",
        artist: "A-Lin",
        title: "Best Song",
      },
      displayResult: "Track=02, Artist=A-Lin, Title=Best Song",
    });
  });

  it("treats regex-looking symbols as literal separators", () => {
    const result = parseFilenameWithConvertPattern(
      "%{track}% [%{version}%] - %{title}%",
      "03 [Live.+] - Song (Final).flac",
    );

    expect(result).toEqual({
      error: "Invalid pattern: Unsupported placeholder: %{version}%",
    });

    expect(
      parseFilenameWithConvertPattern(
        "%{track}% [%{genre}%] - %{title}%",
        "03 [Live.+] - Song (Final).flac",
      ),
    ).toEqual({
      fields: {
        track: "03",
        genre: "Live.+",
        title: "Song (Final)",
      },
      displayResult: "Track=03, Genre=Live.+, Title=Song (Final)",
    });
  });

  it("extracts multiple tags from an existing source tag", () => {
    const source = getConvertSourceValue(track, "title");
    const result = parseTextWithConvertPattern(
      "%{artist}% - %{title}%",
      source,
    );

    expect(result).toEqual({
      fields: {
        artist: "Artist Name",
        title: "Song Title",
      },
      displayResult: "Artist=Artist Name, Title=Song Title",
    });
  });

  it("builds filenames from editable placeholder templates", () => {
    expect(
      buildFilenameFromConvertPattern("%{disc}%-%{track}% %{title}%{ext}%", {
        ...track,
        title: "Clean Song",
      }),
    ).toBe("02-01 Clean Song.flac");
  });

  it("parses tracks in space-separated Chinese filename format", () => {
    const result = parseFilenameWithConvertPattern(
      "%{track}% %{title}%",
      "01 假如梦儿是真的.flac",
    );

    expect(result).toEqual({
      fields: {
        track: "01",
        title: "假如梦儿是真的",
      },
      displayResult: "Track=01, Title=假如梦儿是真的",
    });
  });

  it("rejects patterns without placeholders instead of accepting raw regex", () => {
    expect(parseTextWithConvertPattern("^(?<title>.+)$", "Song")).toEqual({
      error: "Invalid pattern: Pattern must include at least one placeholder like %{title}%",
    });
  });
});
