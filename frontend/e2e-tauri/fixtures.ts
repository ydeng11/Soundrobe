import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import zlib from "node:zlib";

export interface E2eManifest {
  root: string;
  library: string;
  workflowAlbum: string;
  workflowTrack: string;
  incomingAlbum: string;
  incomingTracks: string[];
  auditAlbum: string;
  auditTrack: string;
  autoTagAlbum: string;
  autoTagTrack: string;
  convertAlbum: string;
  convertTrack: string;
  numberAlbum: string;
  numberTracks: string[];
  assistantRepairAlbum: string;
  assistantRepairTracks: string[];
}

export interface E2eWorkspace {
  root: string;
  manifest: E2eManifest;
}

function createFlacWithComments(filePath: string, comments: string[]): void {
  const streamInfo = Buffer.alloc(34);
  streamInfo.writeUInt16BE(4096, 0);
  streamInfo.writeUInt16BE(4096, 2);
  const sampleRate = 44_100;
  const totalSamples = BigInt(sampleRate * 30);
  const packed =
    (BigInt(sampleRate) << 44n) | (1n << 41n) | (15n << 36n) | totalSamples;
  for (let index = 0; index < 8; index += 1) {
    streamInfo[10 + index] = Number(
      (packed >> BigInt((7 - index) * 8)) & 0xffn,
    );
  }

  const streamInfoHeader = Buffer.alloc(4);
  streamInfoHeader[0] = 0;
  streamInfoHeader.writeUIntBE(streamInfo.length, 1, 3);

  const vendor = Buffer.from("soundrobe-tauri-e2e", "utf8");
  const vendorLength = Buffer.alloc(4);
  vendorLength.writeUInt32LE(vendor.length);
  const commentCount = Buffer.alloc(4);
  commentCount.writeUInt32LE(comments.length);
  const encodedComments = comments.flatMap((comment) => {
    const value = Buffer.from(comment, "utf8");
    const length = Buffer.alloc(4);
    length.writeUInt32LE(value.length);
    return [length, value];
  });
  const vorbis = Buffer.concat([
    vendorLength,
    vendor,
    commentCount,
    ...encodedComments,
  ]);
  const vorbisHeader = Buffer.alloc(4);
  vorbisHeader[0] = 0x80 | 0x04;
  vorbisHeader.writeUIntBE(vorbis.length, 1, 3);

  fs.writeFileSync(
    filePath,
    Buffer.concat([
      Buffer.from("fLaC", "ascii"),
      streamInfoHeader,
      streamInfo,
      vorbisHeader,
      vorbis,
    ]),
  );
}

/** Create a small valid PNG for e2e cover-art tests. */
function createCoverPng(filePath: string): void {
  const width = 100;
  const height = 100;
  const rawData = Buffer.alloc((width * 3 + 1) * height);
  for (let y = 0; y < height; y++) {
    rawData[y * (width * 3 + 1)] = 0; // filter: None
    for (let x = 0; x < width; x++) {
      const off = y * (width * 3 + 1) + 1 + x * 3;
      rawData[off] = 128;
      rawData[off + 1] = 200;
      rawData[off + 2] = 255;
    }
  }
  const compressed = zlib.deflateSync(rawData);

  function crc32(buf: Buffer): number {
    let crc = 0xffffffff;
    const table = new Int32Array(256);
    for (let i = 0; i < 256; i++) {
      let c = i;
      for (let j = 0; j < 8; j++)
        c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
      table[i] = c;
    }
    for (let i = 0; i < buf.length; i++)
      crc = table[(crc ^ buf[i]) & 0xff] ^ (crc >>> 8);
    return (crc ^ 0xffffffff) >>> 0;
  }

  function pngChunk(type: string, data: Buffer): Buffer {
    const typeB = Buffer.from(type, "ascii");
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length);
    const crcData = Buffer.concat([typeB, data]);
    const c = Buffer.alloc(4);
    c.writeUInt32BE(crc32(crcData));
    return Buffer.concat([len, typeB, data, c]);
  }

  const sig = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8;  // bit depth
  ihdr[9] = 2;  // colour type: RGB
  ihdr[10] = 0; // compression
  ihdr[11] = 0; // filter
  ihdr[12] = 0; // interlace

  const png = Buffer.concat([
    sig,
    pngChunk("IHDR", ihdr),
    pngChunk("IDAT", compressed),
    pngChunk("IEND", Buffer.alloc(0)),
  ]);
  fs.writeFileSync(filePath, png);
}

export function prepareE2eWorkspace(): E2eWorkspace {
  const inherited = process.env.SOUNDROBE_E2E_MANIFEST;
  if (inherited) {
    const manifest = JSON.parse(inherited) as E2eManifest;
    return { root: manifest.root, manifest };
  }

  const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-tauri-e2e-"));
  const home = path.join(root, "home");
  const library = path.join(root, "library");
  const workflowAlbum = path.join(library, "Workflow Artist", "Workflow Album");
  const incomingAlbum = path.join(library, "Incoming");
  const auditAlbum = path.join(library, "Audit Artist", "2020 - Audit Album");
  const autoTagAlbum = path.join(library, "Offline Artist", "Offline Album");
  const convertAlbum = path.join(library, "Convert Artist", "Convert Album");
  const numberAlbum = path.join(library, "Number Artist", "Number Album");
  const assistantRepairAlbum = path.join(library, "Assistant Artist", "Assistant Repair");
  for (const directory of [
    home,
    workflowAlbum,
    incomingAlbum,
    auditAlbum,
    autoTagAlbum,
    convertAlbum,
    numberAlbum,
    assistantRepairAlbum,
  ]) {
    fs.mkdirSync(directory, { recursive: true });
  }
  const appData = path.join(home, ".soundrobe");
  fs.mkdirSync(appData, { recursive: true });
  fs.writeFileSync(
    path.join(appData, "config.yaml"),
    [
      "remote_lookup_enabled: false",
      "discogs_enabled: false",
      "lyrics_download_enabled: false",
      "",
    ].join("\n"),
  );

  const workflowTrack = path.join(workflowAlbum, "01. Workflow One.flac");
  createFlacWithComments(workflowTrack, [
    "TITLE=Workflow One",
    "ARTIST=Workflow Artist",
    "ALBUM=Workflow Album",
    "TRACKNUMBER=1",
    "TRACKTOTAL=2",
    "MOOD=Bright",
  ]);
  createFlacWithComments(path.join(workflowAlbum, "02. Workflow Two.flac"), [
    "TITLE=Workflow Two",
    "ARTIST=Workflow Artist",
    "ALBUM=Workflow Album",
    "TRACKNUMBER=2",
    "TRACKTOTAL=2",
  ]);

  const incomingTracks = ["Incoming One", "Incoming Two"].map((title, index) => {
    const filePath = path.join(incomingAlbum, `0${index + 1}. ${title}.flac`);
    createFlacWithComments(filePath, [
      `TITLE=${title}`,
      "ARTIST=Grouped Artist",
      "ALBUM=Grouped Album",
      `TRACKNUMBER=${index + 1}`,
      "TRACKTOTAL=2",
    ]);
    return filePath;
  });

  const auditTrack = path.join(auditAlbum, "01. Song.flac");
  createFlacWithComments(auditTrack, [
    "TITLE=Wrong",
    "ARTIST=Audit Artist",
    "ALBUM=Wrong Album",
    "ALBUMARTIST=Audit Artist",
    "DATE=2019",
    "TRACKNUMBER=9",
  ]);

  const autoTagTrack = path.join(autoTagAlbum, "01. Offline Song.flac");
  createFlacWithComments(autoTagTrack, [
    "TITLE=Offline Song",
    "TRACKNUMBER=1",
  ]);

  const convertTrack = path.join(convertAlbum, "01. Convert Song.flac");
  createFlacWithComments(convertTrack, [
    "TITLE=E2E Artist - E2E Song",
    "ARTIST=Old Artist",
    "ALBUM=Convert Album",
    "TRACKNUMBER=1",
  ]);

  const numberTracks = ["03-third", "01-first", "02-second"].map(
    (filename, index) => {
      const filePath = path.join(numberAlbum, `${filename}.flac`);
      createFlacWithComments(filePath, [
        `TITLE=${["Third", "First", "Second"][index]}`,
        "ARTIST=Number Artist",
        "ALBUM=Number Album",
        "TRACKNUMBER=9",
        "TRACKTOTAL=10",
      ]);
      return filePath;
    },
  );

  const assistantRepairTracks = Array.from({ length: 46 }, (_, index) => {
    const number = index + 1;
    const collaborator = `Collaborator ${number}`;
    const filePath = path.join(
      assistantRepairAlbum,
      `${String(number).padStart(2, "0")}. Collaboration ${number}.flac`,
    );
    const artists =
      index >= 44
        ? [`ARTISTS=Artist A & ${collaborator}`]
        : ["ARTISTS=Artist A", `ARTISTS=${collaborator}`];
    createFlacWithComments(filePath, [
      `TITLE=Collaboration ${number}`,
      `ARTIST=Artist A & ${collaborator}`,
      ...artists,
      "ALBUM=Assistant Repair",
      `TRACKNUMBER=${number}`,
      "TRACKTOTAL=46",
    ]);
    return filePath;
  });

  // Write a cover image so the Remove button appears in the right panel
  createCoverPng(path.join(workflowAlbum, "cover.png"));

  const manifest: E2eManifest = {
    root,
    library,
    workflowAlbum,
    workflowTrack,
    incomingAlbum,
    incomingTracks,
    auditAlbum,
    auditTrack,
    autoTagAlbum,
    autoTagTrack,
    convertAlbum,
    convertTrack,
    numberAlbum,
    numberTracks,
    assistantRepairAlbum,
    assistantRepairTracks,
  };
  const xdgConfig = path.join(home, ".config");
  const xdgCache = path.join(home, ".cache");
  fs.mkdirSync(xdgConfig, { recursive: true });
  fs.mkdirSync(xdgCache, { recursive: true });
  Object.assign(process.env, {
    HOME: home,
    USERPROFILE: home,
    APPDATA: path.join(home, "AppData", "Roaming"),
    LOCALAPPDATA: path.join(home, "AppData", "Local"),
    XDG_CONFIG_HOME: xdgConfig,
    XDG_CACHE_HOME: xdgCache,
    SOUNDROBE_E2E_LIBRARY_PATH: library,
    SOUNDROBE_E2E_TRACK_CONTEXT_ACTION: "extra-tags",
    SOUNDROBE_E2E_MANIFEST: JSON.stringify(manifest),
  });
  return { root, manifest };
}

export function cleanupE2eWorkspace(root: string): void {
  // WebView can release its profile files just after the WDIO session ends.
  // Retry the bounded cleanup so transient Windows locks do not fail the run.
  try {
    fs.rmSync(root, {
      recursive: true,
      force: true,
      maxRetries: 10,
      retryDelay: 250,
    });
  } catch (error) {
    const code =
      error && typeof error === "object" && "code" in error
        ? (error as { code?: string }).code
        : undefined;
    if (process.platform === "win32" && code === "EBUSY") {
      console.warn(
        `Windows WebView still owns part of the E2E workspace; leaving it for the runner to reclaim: ${root}`,
      );
      return;
    }
    throw error;
  }
}
