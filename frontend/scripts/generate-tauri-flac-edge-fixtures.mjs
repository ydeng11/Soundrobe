import fs from "node:fs";
import path from "node:path";

const root = path.resolve("test/fixtures/tauri/writer-corpus");
const source = fs.readFileSync(path.join(root, "padded.flac"));

function parse(bytes) {
  if (bytes.subarray(0, 4).toString("ascii") !== "fLaC") throw new Error("source is not FLAC");
  const blocks = [];
  let offset = 4;
  for (;;) {
    if (offset + 4 > bytes.length) throw new Error("truncated FLAC metadata");
    const type = bytes[offset] & 0x7f;
    const last = !!(bytes[offset] & 0x80);
    const length = bytes.readUIntBE(offset + 1, 3);
    const end = offset + 4 + length;
    if (end > bytes.length) throw new Error("truncated FLAC block");
    blocks.push({ type, data: Buffer.from(bytes.subarray(offset + 4, end)) });
    offset = end;
    if (last) return { blocks, audio: Buffer.from(bytes.subarray(offset)) };
  }
}

function serialize(blocks, audio) {
  const parts = [Buffer.from("fLaC")];
  blocks.forEach((block, index) => {
    const header = Buffer.alloc(4);
    header[0] = block.type | (index === blocks.length - 1 ? 0x80 : 0);
    header.writeUIntBE(block.data.length, 1, 3);
    parts.push(header, block.data);
  });
  parts.push(audio);
  return Buffer.concat(parts);
}

const { blocks, audio } = parse(source);
const withoutPadding = blocks.filter((block) => block.type !== 1);
fs.writeFileSync(
  path.join(root, "flac-bare.flac"),
  serialize(blocks.filter((block) => block.type !== 4), audio),
);
fs.writeFileSync(
  path.join(root, "flac-insufficient-padding.flac"),
  serialize([...withoutPadding, { type: 1, data: Buffer.alloc(8) }], audio),
);

const vorbis = blocks.find((block) => block.type === 4);
if (!vorbis) throw new Error("padded source has no Vorbis comment block");
const duplicateAt = withoutPadding.findIndex((block) => block.type === 4);
const duplicate = withoutPadding.slice();
duplicate.splice(duplicateAt + 1, 0, { type: 4, data: Buffer.from(vorbis.data) });
fs.writeFileSync(path.join(root, "flac-duplicate-vc.flac"), serialize(duplicate, audio));

// This fixture reproduces metadata emitted by pre-Soundrobe releases.
const ghostVendor = Buffer.from("auto-tagger", "utf8");
const ghostEntry = Buffer.from("TITLE=GhostTitle", "utf8");
const ghost = Buffer.alloc(4 + ghostVendor.length + 4 + 4 + ghostEntry.length);
ghost.writeUInt32LE(ghostVendor.length, 0);
ghostVendor.copy(ghost, 4);
ghost.writeUInt32LE(1, 4 + ghostVendor.length);
ghost.writeUInt32LE(ghostEntry.length, 8 + ghostVendor.length);
ghostEntry.copy(ghost, 12 + ghostVendor.length);
fs.writeFileSync(path.join(root, "flac-ghost-vc.flac"), Buffer.concat([source, ghost]));

const value = Buffer.from("Wrong APE Album", "utf8");
const key = Buffer.from("ALBUM\0", "ascii");
const item = Buffer.alloc(8 + key.length + value.length);
item.writeUInt32LE(value.length, 0);
key.copy(item, 8);
value.copy(item, 8 + key.length);
const footer = Buffer.alloc(32);
footer.write("APETAGEX", 0, 8, "ascii");
footer.writeUInt32LE(2000, 8);
footer.writeUInt32LE(item.length + footer.length, 12);
footer.writeUInt32LE(1, 16);
fs.writeFileSync(path.join(root, "flac-trailing-ape.flac"), Buffer.concat([source, item, footer]));

console.log("generated deterministic FLAC edge fixtures");
