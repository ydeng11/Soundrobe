import { describe, it, expect } from "vitest";
import {
  buildSelectionMessages,
  buildFallbackMessages,
  buildFolderExtractionMessages,
  buildTagCorrectionMessages,
  buildAuditMessages,
} from "../../electron/handlers/prompts";
import {
  makeLookupRequest,
  makeAlbumCandidate,
  makeTrackCandidate,
} from "../../electron/handlers/candidates";

describe("buildSelectionMessages", () => {
  it("includes request hints and candidates", () => {
    const req = makeLookupRequest({
      artistHint: "Beatles",
      albumHint: "Abbey Road",
      tracks: [
        makeTrackCandidate({ title: "Come Together" }),
        makeTrackCandidate({ title: "Something" }),
      ],
    });

    const candidates = [
      makeAlbumCandidate({
        artist: "The Beatles",
        album: "Abbey Road",
        year: "1969",
        tracks: [
          makeTrackCandidate({ title: "Come Together" }),
          makeTrackCandidate({ title: "Something" }),
        ],
        source: "musicbrainz",
      }),
    ];

    const messages = buildSelectionMessages(req, candidates);
    expect(messages).toHaveLength(2);
    expect(messages[0].role).toBe("system");
    expect(messages[1].role).toBe("user");

    const content = JSON.parse(messages[1].content);
    expect(content.artist_hint).toBe("Beatles");
    expect(content.album_hint).toBe("Abbey Road");
    expect(content.candidates).toHaveLength(1);
    expect(content.candidates[0].artist).toBe("The Beatles");
  });

  it("respects maxCandidates", () => {
    const req = makeLookupRequest({ artistHint: "A" });
    const candidates = Array.from({ length: 10 }, (_, i) =>
      makeAlbumCandidate({ artist: `Artist ${i}`, album: `Album ${i}` }),
    );

    const messages = buildSelectionMessages(req, candidates, 3);
    const content = JSON.parse(messages[1].content);
    expect(content.candidates).toHaveLength(3);
  });
});

describe("buildFallbackMessages", () => {
  it("includes folder candidate and current tracks", () => {
    const req = makeLookupRequest({
      artistHint: "Artist",
      albumHint: "Album",
    });
    const folderCandidate = makeAlbumCandidate({
      artist: "Artist",
      album: "Album",
    });
    const currentTracks = [
      { title: "Track 1", artist: "Artist" },
    ];

    const messages = buildFallbackMessages(req, folderCandidate, currentTracks);
    expect(messages).toHaveLength(2);
    const content = JSON.parse(messages[1].content);
    expect(content.artist_hint).toBe("Artist");
    expect(content.folder_candidate).toBeDefined();
    expect(content.current_tracks).toHaveLength(1);
  });
});

describe("buildFolderExtractionMessages", () => {
  it("includes folder name", () => {
    const messages = buildFolderExtractionMessages(
      "2007-《Goodbye & Hello》[FLAC]",
      "蔡健雅",
    );
    expect(messages).toHaveLength(2);
    const content = JSON.parse(messages[1].content);
    expect(content.folder_name).toBe("2007-《Goodbye & Hello》[FLAC]");
    expect(content.parent_name).toBe("蔡健雅");
  });

  it("omits parentName when null", () => {
    const messages = buildFolderExtractionMessages("Album", null);
    const content = JSON.parse(messages[1].content);
    expect(content.folder_name).toBe("Album");
    expect(content.parent_name).toBeUndefined();
  });
});

describe("buildTagCorrectionMessages", () => {
  it("includes folder name, parent name, parsed hints, and current tracks", () => {
    const messages = buildTagCorrectionMessages(
      "2009-Winter Sweet[flac]",
      "蛋堡",
      "蛋堡",
      "Winter Sweet",
      "2009",
      [
        { title: "Winter Sweet", artist: "蛋堡", trackNumber: 1 },
        { title: "慢慢来", artist: "蛋堡", trackNumber: 2 },
      ],
    );

    expect(messages).toHaveLength(2);
    expect(messages[0].role).toBe("system");
    expect(messages[1].role).toBe("user");

    const content = JSON.parse(messages[1].content);
    expect(content.folder_name).toBe("2009-Winter Sweet[flac]");
    expect(content.parent_name).toBe("蛋堡");
    expect(content.parsed_hints).toEqual({
      artist: "蛋堡",
      album: "Winter Sweet",
      year: "2009",
    });
    expect(content.current_tracks).toHaveLength(2);
    expect(content.current_tracks[0].title).toBe("Winter Sweet");
    expect(content.current_tracks[0].track_number).toBe(1);
  });

  it("omits parent_name when not provided", () => {
    const messages = buildTagCorrectionMessages(
      "Album[flac]",
      null,
      "Artist",
      "Album",
      null,
      [],
    );
    const content = JSON.parse(messages[1].content);
    expect(content.parent_name).toBeUndefined();
  });

  it("omits current_tracks when empty", () => {
    const messages = buildTagCorrectionMessages(
      "Album",
      "Artist",
      "Artist",
      "Album",
      "2020",
      [],
    );
    const content = JSON.parse(messages[1].content);
    expect(content.current_tracks).toBeUndefined();
  });

  it("includes system instructions asking for artist, albumArtist, album, year, genre, tracks, confidence", () => {
    const messages = buildTagCorrectionMessages(
      "Album",
      "Artist",
      "Artist",
      "Album",
      null,
      [],
    );
    const systemMsg = messages[0].content;

    expect(systemMsg).toContain("artist");
    expect(systemMsg).toContain("albumArtist");
    expect(systemMsg).toContain("album");
    expect(systemMsg).toContain("year");
    expect(systemMsg).toContain("genre");
    expect(systemMsg).toContain("tracks");
    expect(systemMsg).toContain("confidence");
  });

  it("includes full_path and filenames when context is provided", () => {
    const messages = buildTagCorrectionMessages(
      "2009-100天",
      "林俊杰",
      "林俊杰",
      "0天",
      "2009",
      [{ title: "X", artist: "林俊杰" }],
      {
        fullPath: "/Volumes/downloads/林俊杰/2009-100天",
        filenames: ["林俊杰 - 01.X.flac", "林俊杰 - 02.第几个100天.flac"],
        existingAlbumTags: ["0天"],
        existingArtistTags: ["林俊杰"],
      },
    );
    const payload = JSON.parse(messages[1].content);
    expect(payload.full_path).toBe("/Volumes/downloads/林俊杰/2009-100天");
    expect(payload.filenames).toEqual(["林俊杰 - 01.X.flac", "林俊杰 - 02.第几个100天.flac"]);
    expect(payload.existing_album_tags).toEqual(["0天"]);
    expect(payload.existing_artist_tags).toEqual(["林俊杰"]);
  });

  it("omits context fields when not provided", () => {
    const messages = buildTagCorrectionMessages(
      "Album",
      "Artist",
      "Artist",
      "Album",
      null,
      [],
    );
    const payload = JSON.parse(messages[1].content);
    expect(payload.full_path).toBeUndefined();
    expect(payload.filenames).toBeUndefined();
    expect(payload.existing_album_tags).toBeUndefined();
    expect(payload.existing_artist_tags).toBeUndefined();
  });

  it("system prompt contains rule about year prefix misparsing", () => {
    const messages = buildTagCorrectionMessages(
      "Album",
      null,
      null,
      null,
      null,
      [],
    );
    expect(messages[0].content).toContain("parsed_hints may be WRONG");
    expect(messages[0].content).toContain("2009-100天");
  });

  it("system prompt contains rule for Year-Artist-Album separator patterns", () => {
    const messages = buildTagCorrectionMessages(
      "Album",
      null,
      null,
      null,
      null,
      [],
    );
    expect(messages[0].content).toContain("Year - Artist - Album");
    expect(messages[0].content).toContain("Lossless");
    expect(messages[0].content).toContain("24bit-48Hz");
  });
});

describe("buildAuditMessages", () => {
  it("builds a targeted audit payload and requires per-field confidence", () => {
    const messages = buildAuditMessages(
      "Artist",
      "Album",
      [
        {
          title: "Song",
          artist: "Artist",
          artists: ["Artist"],
          album: "Album",
          albumArtist: "Artist",
          year: "2020",
          genre: null,
        },
      ],
      ["01. Song.flac"],
      {
        reviewTargets: [
          {
            index: 0,
            field: "genre",
            current: "",
            evidence: "Genre tag is empty.",
            reason: "Semantic genre requires LLM judgment.",
          },
        ],
      },
    );

    expect(messages[0].content).toContain("one field at a time");
    expect(messages[0].content).toContain("confidence");
    expect(messages[0].content).not.toContain("complete corrected metadata");

    const payload = JSON.parse(messages[1].content);
    expect(payload.tracks).toEqual([
      expect.objectContaining({
        index: 0,
        path: "01. Song.flac",
        title: "Song",
        genre: "",
      }),
    ]);
    expect(payload.review_targets).toEqual([
      expect.objectContaining({
        index: 0,
        field: "genre",
      }),
    ]);
    expect(payload.review_targets).not.toEqual(expect.arrayContaining([
      expect.objectContaining({ field: "title" }),
      expect.objectContaining({ field: "album" }),
    ]));
  });
});
