import { describe, it, expect, beforeEach, afterEach } from "vitest";
import fs from "fs";
import path from "path";
import os from "os";
import * as NodeID3 from "node-id3";
import { parseFile } from "music-metadata";
import { writeTags } from "../../electron/handlers/writer";
import { readTrackMetadata, readExtraTags } from "../../electron/handlers/tracks";

/**
 * Create a minimal valid MP3 file with ID3v2 tags using node-id3,
 * then append a minimal MPEG sync frame (417 bytes) so music-metadata
 * recognizes it as audio.
 */
function createMinimalMp3(
  filePath: string,
  initialTags?: Record<string, string>,
): void {
  if (initialTags) {
    NodeID3.write(
      {
        title: initialTags.title,
        artist: initialTags.artist,
        album: initialTags.album,
        year: initialTags.year,
        genre: initialTags.genre,
        trackNumber: initialTags.trackNumber
          ? parseInt(initialTags.trackNumber, 10)
          : undefined,
      },
      filePath,
    );
  } else {
    NodeID3.write({}, filePath);
  }

  const fd = fs.openSync(filePath, "a");
  const frame = Buffer.alloc(417);
  frame[0] = 0xff;
  frame[1] = 0xfb;
  frame[2] = (9 << 4) | (0 << 2);
  frame[3] = 0x02;
  fs.writeSync(fd, frame, 0, frame.length);
  fs.closeSync(fd);
}

/**
 * Create a minimal FLAC file with Vorbis comments.
 */
function createMinimalFlac(
  filePath: string,
  title?: string,
  artist?: string,
  album?: string,
): void {
  const parts: Buffer[] = [];
  parts.push(Buffer.from("fLaC", "ascii"));

  // STREAMINFO
  const si = Buffer.alloc(34);
  si.writeUInt16BE(4096, 0);
  si.writeUInt16BE(4096, 2);
  si[12] = 0x00;
  si[13] = 0xac;
  si[14] = 0x44;
  si[15] = 0x02;
  si[16] = 0x1f;

  const siHeader = Buffer.alloc(4);
  const hasVorbis = !!(title || artist || album);
  siHeader[0] = hasVorbis ? 0x00 : 0x80;
  siHeader[1] = (si.length >> 16) & 0xff;
  siHeader[2] = (si.length >> 8) & 0xff;
  siHeader[3] = si.length & 0xff;
  parts.push(siHeader, si);

  if (hasVorbis) {
    const comments: string[] = [];
    if (title) comments.push(`TITLE=${title}`);
    if (artist) comments.push(`ARTIST=${artist}`);
    if (album) comments.push(`ALBUM=${album}`);

    const vendor = Buffer.from("libFLAC 1.3.2", "utf8");
    const vLen = Buffer.alloc(4);
    vLen.writeUInt32LE(vendor.length);

    const cBufs: Buffer[] = [];
    for (const c of comments) {
      const cb = Buffer.from(c, "utf8");
      const cl = Buffer.alloc(4);
      cl.writeUInt32LE(cb.length);
      cBufs.push(cl, cb);
    }

    const n = Buffer.alloc(4);
    n.writeUInt32LE(comments.length);

    const vb = Buffer.concat([vLen, vendor, n, ...cBufs]);
    const vh = Buffer.alloc(4);
    vh[0] = 0x80 | 0x04;
    vh[1] = (vb.length >> 16) & 0xff;
    vh[2] = (vb.length >> 8) & 0xff;
    vh[3] = vb.length & 0xff;
    parts.push(vh, vb);
  }

  fs.writeFileSync(filePath, Buffer.concat(parts));
}

describe("writeTags — Discogs IDs round-trip (MP3)", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "writer-discogs-mp3-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("writes and reads back Discogs IDs from MP3", async () => {
    const fp = path.join(tmpDir, "test.mp3");
    createMinimalMp3(fp);

    await writeTags(fp, {
      title: "Test Song",
      artist: "Hedgehog",
      discogsArtistId: "1902728",
      discogsReleaseId: "6951078",
    });

    // Verify via node-id3 native read
    const tags = NodeID3.read(fp);
    const udt = tags.userDefinedText ?? [];
    const artistId = (Array.isArray(udt) ? udt : [udt]).find(
      (r: any) => r.description === "Discogs Artist Id",
    );
    const releaseId = (Array.isArray(udt) ? udt : [udt]).find(
      (r: any) => r.description === "Discogs Release Id",
    );
    expect(artistId?.value).toBe("1902728");
    expect(releaseId?.value).toBe("6951078");
  });

  it("reads Discogs IDs back via readExtraTags on MP3", async () => {
    const fp = path.join(tmpDir, "test.mp3");
    createMinimalMp3(fp);

    await writeTags(fp, {
      discogsArtistId: "1902728",
      discogsReleaseId: "6951078",
    });

    const extras = await readExtraTags(fp);
    const artistId = extras.find((t) => t.key === "Discogs Artist Id");
    const releaseId = extras.find((t) => t.key === "Discogs Release Id");
    expect(artistId?.value).toBe("1902728");
    expect(releaseId?.value).toBe("6951078");
  });

  it("reads MusicBrainz IDs back via readTrackMetadata on MP3", async () => {
    const fp = path.join(tmpDir, "test.mp3");
    createMinimalMp3(fp);

    await writeTags(fp, {
      title: "Test",
      musicbrainzTrackId: "mb-track-1",
      musicbrainzAlbumId: "mb-album-1",
      musicbrainzArtistId: "mb-artist-1",
    });

    const meta = await readTrackMetadata(fp);
    expect(meta.musicbrainzTrackId).toBe("mb-track-1");
    expect(meta.musicbrainzAlbumId).toBe("mb-album-1");
    expect(meta.musicbrainzArtistId).toBe("mb-artist-1");
  });

  it("reads Discogs IDs back via readTrackMetadata on MP3", async () => {
    const fp = path.join(tmpDir, "test.mp3");
    createMinimalMp3(fp);

    await writeTags(fp, {
      title: "Test",
      discogsArtistId: "1902728",
      discogsReleaseId: "6951078",
    });

    const meta = await readTrackMetadata(fp);
    expect(meta.discogsArtistId).toBe("1902728");
    expect(meta.discogsReleaseId).toBe("6951078");
  });
});

describe("writeTags — Discogs IDs round-trip (FLAC)", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "writer-discogs-flac-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("writes and reads back Discogs IDs from FLAC via metadata", async () => {
    const fp = path.join(tmpDir, "test.flac");
    createMinimalFlac(fp, "Test Song", "Hedgehog", "Album");

    await writeTags(fp, {
      title: "Test Song",
      artist: "Hedgehog",
      discogsArtistId: "1902728",
      discogsReleaseId: "6951078",
    });

    // Verify via music-metadata native tags
    const parsed = await parseFile(fp);
    const native = parsed.native;
    let foundArtistId = false;
    let foundReleaseId = false;
    for (const [, tags] of Object.entries(native)) {
      for (const tag of tags) {
        if (tag.id === "DISCOGS_ARTIST_ID" && typeof tag.value === "string") {
          expect(tag.value).toBe("1902728");
          foundArtistId = true;
        }
        if (tag.id === "DISCOGS_RELEASE_ID" && typeof tag.value === "string") {
          expect(tag.value).toBe("6951078");
          foundReleaseId = true;
        }
      }
    }
    expect(foundArtistId).toBe(true);
    expect(foundReleaseId).toBe(true);
  });

  it("reads Discogs IDs back via readTrackMetadata on FLAC", async () => {
    const fp = path.join(tmpDir, "test.flac");
    createMinimalFlac(fp, "Test", "Hedgehog", "Album");

    await writeTags(fp, {
      title: "Test",
      artist: "Hedgehog",
      discogsArtistId: "1902728",
      discogsReleaseId: "6951078",
    });

    const meta = await readTrackMetadata(fp);
    expect(meta.discogsArtistId).toBe("1902728");
    expect(meta.discogsReleaseId).toBe("6951078");
  });

  it("reads MusicBrainz IDs back via readTrackMetadata on FLAC", async () => {
    const fp = path.join(tmpDir, "test.flac");
    createMinimalFlac(fp, "Test", "Artist", "Album");

    await writeTags(fp, {
      musicbrainzTrackId: "mb-track-1",
      musicbrainzAlbumId: "mb-album-1",
      musicbrainzArtistId: "mb-artist-1",
    });

    const meta = await readTrackMetadata(fp);
    expect(meta.musicbrainzTrackId).toBe("mb-track-1");
    expect(meta.musicbrainzAlbumId).toBe("mb-album-1");
    expect(meta.musicbrainzArtistId).toBe("mb-artist-1");
  });

  it("reads Discogs IDs as extra tags on FLAC", async () => {
    const fp = path.join(tmpDir, "test.flac");
    createMinimalFlac(fp);

    await writeTags(fp, {
      discogsArtistId: "1902728",
      discogsReleaseId: "6951078",
    });

    const extras = await readExtraTags(fp);
    const artistId = extras.find((t) => t.key === "DISCOGS_ARTIST_ID");
    const releaseId = extras.find((t) => t.key === "DISCOGS_RELEASE_ID");
    expect(artistId?.value).toBe("1902728");
    expect(releaseId?.value).toBe("6951078");
  });
});
