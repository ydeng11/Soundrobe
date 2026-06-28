import { TextDecoder } from "util";

export interface ApeTagLocation {
  position: number;
  tagSize: number;
  itemCount: number;
  hasHeader: boolean;
  hasFooter: boolean;
  footerOffset: number;
  headerOffset: number | null;
  itemsStart: number;
  itemsEnd: number;
}

export interface ApeItem {
  key: string;
  value: string;
  type: "text";
  readonly: boolean;
}

export interface ApeTag {
  version: number | null;
  hasHeader: boolean;
  hasFooter: boolean;
  items: ApeItem[];
  issues: string[];
}

const APE_PREAMBLE = Buffer.from("APETAGEX", "ascii");
const APE_FOOTER_BYTES = 32;
const APE_HEADER_FLAG = 0x20000000;
const APE_FOOTER_FLAG = 0x80000000;
const APE_ITEM_TYPE_MASK = 0x60000000;
const APE_BINARY_ITEM_TYPE = 0x40000000;
const MAX_APE_TAG_SIZE = 16 * 1024 * 1024;
const MAX_APE_ITEM_COUNT = 100_000;
const utf8Decoder = new TextDecoder("utf-8", { fatal: true });

interface FooterCandidate {
  offset: number;
  version: number;
  tagSize: number;
  itemCount: number;
  flags: number;
  itemsStart: number;
  itemsEnd: number;
  headerOffset: number | null;
}

/**
 * Compute the end of Monkey's Audio data from the MAC descriptor/header.
 * Returns the byte offset where trailing tag metadata can begin.
 */
function computeAudioEnd(data: Buffer): number | null {
  if (data.length < 52 || data.toString("ascii", 0, 4) !== "MAC ") return null;

  const descriptorBytes = data.readUInt32LE(8);
  const headerBytes = data.readUInt32LE(12);
  if (
    descriptorBytes < 52 ||
    headerBytes < 24 ||
    data.length < descriptorBytes + headerBytes
  ) {
    return null;
  }

  const seekTableBytes = data.readUInt32LE(16);
  const headerDataBytes = data.readUInt32LE(20);
  const apeFrameDataBytes = data.readUInt32LE(24);
  const terminatingDataBytes = data.readUInt32LE(32);
  const audioEnd =
    descriptorBytes +
    headerBytes +
    seekTableBytes +
    headerDataBytes +
    apeFrameDataBytes +
    terminatingDataBytes;

  return audioEnd > 0 && audioEnd <= data.length ? audioEnd : null;
}

function findApeSignatures(data: Buffer): number[] {
  const offsets: number[] = [];
  let searchFrom = 0;
  while (searchFrom <= data.length - APE_PREAMBLE.length) {
    const offset = data.indexOf(APE_PREAMBLE, searchFrom);
    if (offset < 0) break;
    offsets.push(offset);
    searchFrom = offset + 1;
  }
  return offsets;
}

function readFooterCandidate(
  data: Buffer,
  offset: number,
  issues?: string[],
): FooterCandidate | null {
  if (offset < 0 || offset + APE_FOOTER_BYTES > data.length) {
    addIssue(issues, "invalid_footer_span");
    return null;
  }
  if (!data.subarray(offset, offset + APE_PREAMBLE.length).equals(APE_PREAMBLE)) {
    return null;
  }

  const version = data.readUInt32LE(offset + 8);
  const tagSize = data.readUInt32LE(offset + 12);
  const itemCount = data.readUInt32LE(offset + 16);
  const flags = data.readUInt32LE(offset + 20);

  if (flags & APE_HEADER_FLAG) return null;
  if (tagSize < APE_FOOTER_BYTES || tagSize > MAX_APE_TAG_SIZE || tagSize > data.length) {
    addIssue(issues, "invalid_tag_size");
    return null;
  }
  if (itemCount > MAX_APE_ITEM_COUNT) {
    addIssue(issues, "invalid_item_count");
    return null;
  }

  const itemsStart = offset + APE_FOOTER_BYTES - tagSize;
  if (itemsStart < 0 || itemsStart > offset) {
    addIssue(issues, "invalid_item_region");
    return null;
  }

  const headerOffset = itemsStart - APE_FOOTER_BYTES;
  const hasHeaderSignature =
    headerOffset >= 0 &&
    headerOffset + APE_FOOTER_BYTES <= data.length &&
    data.subarray(headerOffset, headerOffset + APE_PREAMBLE.length).equals(APE_PREAMBLE);
  const hasHeader =
    hasHeaderSignature &&
    !!(data.readUInt32LE(headerOffset + 20) & APE_HEADER_FLAG) &&
    data.readUInt32LE(headerOffset + 12) === tagSize &&
    data.readUInt32LE(headerOffset + 16) === itemCount;
  if (hasHeaderSignature && !hasHeader) {
    addIssue(issues, "invalid_header_span");
  }

  return {
    offset,
    version,
    tagSize,
    itemCount,
    flags,
    itemsStart,
    itemsEnd: offset,
    headerOffset: hasHeader ? headerOffset : null,
  };
}

function collectFooterCandidates(data: Buffer, issues?: string[]): FooterCandidate[] {
  const candidates: FooterCandidate[] = [];
  const signatures = findApeSignatures(data);
  for (const offset of signatures) {
    const candidate = readFooterCandidate(data, offset, issues);
    if (candidate) candidates.push(candidate);
  }
  if (issues && signatures.length > 0 && candidates.length === 0 && issues.length === 0) {
    issues.push("missing_footer");
  }
  return candidates;
}

export function findApeFooterOffset(data: Buffer): number {
  const candidates = collectFooterCandidates(data);
  return candidates.length > 0 ? candidates[candidates.length - 1].offset : -1;
}

export function locateApeTag(data: Buffer): ApeTagLocation | null {
  return locateApeTagInternal(data);
}

function locateApeTagInternal(data: Buffer, issues?: string[]): ApeTagLocation | null {
  const candidates = collectFooterCandidates(data, issues);
  if (candidates.length === 0) return null;

  const primary = candidates[candidates.length - 1];
  let position = data.length;
  for (const candidate of candidates) {
    const candidateStart = candidate.headerOffset ?? candidate.itemsStart;
    if (candidateStart < position) position = candidateStart;
  }

  const audioEnd = computeAudioEnd(data);
  if (audioEnd !== null && position > audioEnd) {
    position = audioEnd;
  }

  if (position < 0 || position >= data.length) return null;

  return {
    position,
    tagSize: primary.tagSize,
    itemCount: primary.itemCount,
    hasHeader: primary.headerOffset !== null,
    hasFooter: true,
    footerOffset: primary.offset,
    headerOffset: primary.headerOffset,
    itemsStart: primary.itemsStart,
    itemsEnd: primary.itemsEnd,
  };
}

export function stripApeTag(data: Buffer): Buffer {
  const location = locateApeTag(data);
  if (!location) return data;
  return data.subarray(0, location.position);
}

export function buildApeTagItems(
  entries: Array<{ key: string; value: string }>,
): { items: Buffer; count: number } {
  const chunks: Buffer[] = [];
  let count = 0;

  for (const { key, value } of entries) {
    if (!isValidApeKey(key) || value == null || value === "") continue;

    const keyBuf = Buffer.from(`${key.toUpperCase()}\0`, "utf8");
    const valueBuf = Buffer.from(value, "utf8");
    const header = Buffer.alloc(8);
    header.writeUInt32LE(valueBuf.length, 0);
    header.writeUInt32LE(APE_HEADER_FLAG, 4);
    chunks.push(header, keyBuf, valueBuf);
    count++;
  }

  return { items: Buffer.concat(chunks), count };
}

export function buildApeFooter(tagSize: number, itemCount: number): Buffer {
  const footer = Buffer.alloc(APE_FOOTER_BYTES);
  footer.write("APETAGEX", 0, 8, "ascii");
  footer.writeUInt32LE(2000, 8);
  footer.writeUInt32LE(tagSize, 12);
  footer.writeUInt32LE(itemCount, 16);
  footer.writeUInt32LE(APE_FOOTER_FLAG, 20);
  return footer;
}

export function parseApeTag(data: Buffer): ApeTag {
  const empty: ApeTag = {
    version: null,
    hasHeader: false,
    hasFooter: false,
    items: [],
    issues: [],
  };
  const issues: string[] = [];
  const location = locateApeTagInternal(data, issues);
  if (!location) return issues.length > 0 ? { ...empty, issues } : empty;

  const footer = readFooterCandidate(data, location.footerOffset);
  if (!footer) {
    return { ...empty, issues: [...issues, "invalid_footer"] };
  }

  const items: ApeItem[] = [];
  let offset = footer.itemsStart;

  for (let index = 0; index < footer.itemCount; index++) {
    if (offset + 8 > footer.itemsEnd) {
      issues.push("item_out_of_bounds");
      return tagWithIssues(footer, items, issues);
    }

    const valueSize = data.readUInt32LE(offset);
    const flags = data.readUInt32LE(offset + 4);
    offset += 8;

    const nullIndex = data.indexOf(0, offset);
    if (nullIndex < 0 || nullIndex >= footer.itemsEnd) {
      issues.push("missing_key_terminator");
      return tagWithIssues(footer, items, issues);
    }

    const key = decodeUtf8(data.subarray(offset, nullIndex), issues);
    if (key === null) return tagWithIssues(footer, [], issues);
    offset = nullIndex + 1;

    if (!isValidApeKey(key)) {
      issues.push("invalid_key");
      return tagWithIssues(footer, [], issues);
    }

    if (offset + valueSize > footer.itemsEnd) {
      issues.push("item_value_out_of_bounds");
      return tagWithIssues(footer, items, issues);
    }

    const itemType = flags & APE_ITEM_TYPE_MASK;
    const valueBytes = data.subarray(offset, offset + valueSize);
    offset += valueSize;

    if (itemType === APE_BINARY_ITEM_TYPE || itemType === APE_ITEM_TYPE_MASK) {
      issues.push("unsupported_item_type");
      continue;
    }
    if (itemType !== 0 && itemType !== APE_HEADER_FLAG) {
      issues.push("malformed_flags");
      return tagWithIssues(footer, [], issues);
    }

    const value = decodeUtf8(valueBytes, issues);
    if (value === null) return tagWithIssues(footer, [], issues);

    items.push({
      key,
      value,
      type: "text",
      readonly: !!(flags & 0x1),
    });
  }

  if (offset !== footer.itemsEnd) {
    issues.push("trailing_item_bytes");
    return tagWithIssues(footer, [], issues);
  }

  return tagWithIssues(footer, items, issues);
}

export function parseApeTagItems(data: Buffer): Array<{ key: string; value: string }> {
  return parseApeTag(data).items.map(({ key, value }) => ({ key, value }));
}

function tagWithIssues(
  footer: FooterCandidate,
  items: ApeItem[],
  issues: string[],
): ApeTag {
  return {
    version: footer.version,
    hasHeader: footer.headerOffset !== null,
    hasFooter: true,
    items,
    issues,
  };
}

function decodeUtf8(bytes: Buffer, issues: string[]): string | null {
  try {
    return utf8Decoder.decode(bytes);
  } catch {
    issues.push("invalid_utf8");
    return null;
  }
}

function addIssue(issues: string[] | undefined, issue: string): void {
  if (issues && !issues.includes(issue)) issues.push(issue);
}

function isValidApeKey(key: string): boolean {
  if (!key || key.length < 2 || key.length > 255) return false;
  for (const ch of key) {
    const code = ch.charCodeAt(0);
    if (code < 0x20 || code === 0x7f) return false;
  }
  return true;
}
