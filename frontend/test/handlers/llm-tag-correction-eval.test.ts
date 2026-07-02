/**
 * LLM Tag Correction Evaluation Test
 *
 * Tests the LLM's ability to correctly identify album names when the
 * folder-name parser produces wrong hints — especially numeric album
 * names like "100天" that the regex misparses as "0天".
 *
 * This test calls the REAL LLM API. It is skipped unless both:
 *   RUN_LLM_EVAL=1  AND  LLM_API_KEY=...
 *
 * Run:
 *   RUN_LLM_EVAL=1 LLM_API_KEY=... npx vitest run test/handlers/llm-tag-correction-eval.test.ts
 */

import { describe, it, expect, beforeAll } from "vitest";
import { buildTagCorrectionMessages } from "../../electron/handlers/prompts";
import { OpenRouterClient } from "../../electron/handlers/openrouter";

// ── Configuration ──────────────────────────────────────────────────

const SHOULD_RUN =
  process.env.RUN_LLM_EVAL === "1" && !!process.env.LLM_API_KEY;

const LLM_API_KEY = process.env.LLM_API_KEY ?? "";
const LLM_MODEL = process.env.LLM_MODEL ?? "openrouter/owl-alpha";

const client = SHOULD_RUN
  ? new OpenRouterClient({ apiKey: LLM_API_KEY, model: LLM_MODEL })
  : null;

// Skip entire suite when env vars are not set
const describeLlm = SHOULD_RUN ? describe : describe.skip;

// LLM tests need more time — the API can take 10-15s per call
const LLM_TIMEOUT = 30_000;

// ── Schema ─────────────────────────────────────────────────────────

const TAG_CORRECTION_SCHEMA = {
  type: "object",
  properties: {
    artist: { type: "string" },
    albumArtist: { type: "string" },
    album: { type: "string" },
    year: { type: "string" },
    genre: { type: "string" },
    tracks: {
      type: "array",
      items: {
        type: "object",
        properties: {
          index: { type: "number" },
          title: { type: "string" },
          artist: { type: "string" },
        },
      },
    },
    confidence: { type: "number" },
  },
};

// ── Test cases ─────────────────────────────────────────────────────

interface LlmEvalCase {
  name: string;
  folderName: string;
  parentName: string | null;
  parsedArtist: string | null;
  parsedAlbum: string | null;
  parsedYear: string | null;
  tracks: Array<{
    title: string;
    artist: string;
    album?: string | null;
    trackNumber?: number | null;
    genre?: string | null;
  }>;
  expectedAlbum: string;
}

const TEST_CASES: LlmEvalCase[] = [
  // ── Group A: Numeric album names with year prefix (parser gets wrong) ──
  {
    name: "A1: 100天 with year prefix",
    folderName: "2009-100天",
    parentName: "林俊杰",
    parsedArtist: "林俊杰",
    parsedAlbum: "0天", // WRONG
    parsedYear: "2009",
    tracks: [
      { title: "X", artist: "林俊杰" },
      { title: "第几个100天", artist: "林俊杰" },
    ],
    expectedAlbum: "100天",
  },
  {
    name: "A2: 200首经典 with year prefix",
    folderName: "2005-200首经典",
    parentName: "群星",
    parsedArtist: "群星",
    parsedAlbum: "0首经典", // WRONG
    parsedYear: "2005",
    tracks: [
      { title: "歌曲一", artist: "群星" },
      { title: "歌曲二", artist: "群星" },
    ],
    expectedAlbum: "200首经典",
  },
  {
    name: "A3: 1001夜 with year prefix",
    folderName: "2017-1001夜",
    parentName: "周深",
    parsedArtist: "周深",
    parsedAlbum: "1夜", // WRONG
    parsedYear: "2017",
    tracks: [{ title: "大鱼", artist: "周深" }],
    expectedAlbum: "1001夜",
  },
  {
    name: "A4: 10秒学会日语 with year prefix",
    folderName: "2005-10秒学会日语",
    parentName: null,
    parsedArtist: null,
    parsedAlbum: "0秒学会日语", // WRONG
    parsedYear: "2005",
    tracks: [{ title: "第一课", artist: null }],
    expectedAlbum: "10秒学会日语",
  },
  {
    name: "A5: 1st album with year prefix",
    folderName: "2010-1st album",
    parentName: "Artist",
    parsedArtist: "Artist",
    parsedAlbum: "st album", // WRONG
    parsedYear: "2010",
    tracks: [{ title: "Song", artist: "Artist" }],
    expectedAlbum: "1st album",
  },
  {
    name: "A6: 100 (numeric only) with year prefix",
    folderName: "2015-100",
    parentName: "Artist",
    parsedArtist: "Artist",
    parsedAlbum: "", // WRONG
    parsedYear: "2015",
    tracks: [{ title: "Track", artist: "Artist" }],
    expectedAlbum: "100",
  },
  {
    name: "A7: 300 with year prefix",
    folderName: "2006-300",
    parentName: "Artist",
    parsedArtist: "Artist",
    parsedAlbum: "0", // WRONG
    parsedYear: "2006",
    tracks: [{ title: "Track", artist: "Artist" }],
    expectedAlbum: "300",
  },

  // ── Group B: Normal date prefixes (parser gets right) ──
  {
    name: "B1: normal 2009-04 Something",
    folderName: "2009-04 Something",
    parentName: "Artist",
    parsedArtist: "Artist",
    parsedAlbum: "Something",
    parsedYear: "2009",
    tracks: [{ title: "Track 1", artist: "Artist" }],
    expectedAlbum: "Something",
  },
  {
    name: "B2: full date F.I.R album",
    folderName: "2007-09-28 F.I.R飞儿乐团 爱‧歌姬(24bit-48Hz)(WAV)",
    parentName: "F.I.R.",
    parsedArtist: "F.I.R.",
    parsedAlbum: "F.I.R飞儿乐团 爱‧歌姬(24bit-48Hz)",
    parsedYear: "2007",
    tracks: [{ title: "Lydia", artist: "F.I.R飞儿乐团" }],
    expectedAlbum: "爱‧歌姬",
  },
  {
    name: "B3: CJK dotted album",
    folderName: "1992-跳不完.爱不完.唱不完",
    parentName: "郭富城",
    parsedArtist: "郭富城",
    parsedAlbum: "跳不完.爱不完.唱不完",
    parsedYear: "1992",
    tracks: [{ title: "跳不完", artist: "郭富城" }],
    expectedAlbum: "跳不完.爱不完.唱不完",
  },
  {
    name: "B4: album is the year (1984)",
    folderName: "2020-1984",
    parentName: "George Orwell",
    parsedArtist: "George Orwell",
    parsedAlbum: "1984",
    parsedYear: "2020",
    tracks: [{ title: "Part 1", artist: "George Orwell" }],
    expectedAlbum: "1984",
  },
  {
    name: "B5: no year prefix at all",
    folderName: "100天",
    parentName: "林俊杰",
    parsedArtist: "林俊杰",
    parsedAlbum: "100天",
    parsedYear: null,
    tracks: [{ title: "X", artist: "林俊杰" }],
    expectedAlbum: "100天",
  },

  // ── Group C: Various separators and format suffixes ──
  {
    name: "C1: dash separator Year - Artist - Album",
    folderName: "2009 - 林俊杰 - 100天",
    parentName: "林俊杰",
    parsedArtist: "林俊杰",
    parsedAlbum: "100天",
    parsedYear: "2009",
    tracks: [{ title: "X", artist: "林俊杰" }],
    expectedAlbum: "100天",
  },
  {
    name: "C2: dash separator with (Lossless)",
    folderName: "2009 - 林俊杰 - 100天 (Lossless)",
    parentName: "林俊杰",
    parsedArtist: "林俊杰",
    parsedAlbum: "100天",
    parsedYear: "2009",
    tracks: [{ title: "X", artist: "林俊杰" }],
    expectedAlbum: "100天",
  },
  {
    name: "C3: comma separator with [24bit]",
    folderName: "2009, 林俊杰, 100天 [24bit]",
    parentName: "林俊杰",
    parsedArtist: "林俊杰",
    parsedAlbum: "100天",
    parsedYear: "2009",
    tracks: [{ title: "X", artist: "林俊杰" }],
    expectedAlbum: "100天",
  },
  {
    name: "C4: space+underscore separator with (24bit-48Hz)(WAV)",
    folderName: "2009 林俊杰_100天 (24bit-48Hz)(WAV)",
    parentName: "林俊杰",
    parsedArtist: "林俊杰",
    parsedAlbum: "100天",
    parsedYear: "2009",
    tracks: [{ title: "X", artist: "林俊杰" }],
    expectedAlbum: "100天",
  },
  {
    name: "C5: dash separator with (经典)",
    folderName: "2009-林俊杰-100天(经典)",
    parentName: "林俊杰",
    parsedArtist: "林俊杰",
    parsedAlbum: "100天",
    parsedYear: "2009",
    tracks: [{ title: "X", artist: "林俊杰" }],
    expectedAlbum: "100天",
  },
  {
    name: "C6: dot+space separator with [FLAC]",
    folderName: "2009.04 林俊杰 - 100天[FLAC]",
    parentName: "林俊杰",
    parsedArtist: "林俊杰",
    parsedAlbum: "100天",
    parsedYear: "2009",
    tracks: [{ title: "X", artist: "林俊杰" }],
    expectedAlbum: "100天",
  },
  {
    name: "C7: no separator YearArtistAlbum",
    folderName: "2009林俊杰100天",
    parentName: "林俊杰",
    parsedArtist: "林俊杰",
    parsedAlbum: "100天",
    parsedYear: "2009",
    tracks: [{ title: "X", artist: "林俊杰" }],
    expectedAlbum: "100天",
  },
  {
    name: "C8: dash separator with [24B/48H]",
    folderName: "2009 - 林俊杰 - 100天 [24B/48H]",
    parentName: "林俊杰",
    parsedArtist: "林俊杰",
    parsedAlbum: "100天",
    parsedYear: "2009",
    tracks: [{ title: "X", artist: "林俊杰" }],
    expectedAlbum: "100天",
  },

  // ── Group D: Real-world cases — title contains artist suffix ──
  {
    name: "D1: 美妙生活 — titles have artist suffix",
    folderName: "2011-美妙生活",
    parentName: "林宥嘉",
    parsedArtist: "林宥嘉",
    parsedAlbum: "美妙生活",
    parsedYear: "2011",
    tracks: [
      { title: "Fly My Way", artist: "林宥嘉" },
      { title: "不换-林宥嘉", artist: "林宥嘉" },
      { title: "想念-林宥嘉", artist: "林宥嘉" },
      { title: "想自由-林宥嘉", artist: "林宥嘉" },
      { title: "我总是一个人在练习一个人-林宥嘉", artist: "林宥嘉" },
      { title: "拥有-林宥嘉", artist: "林宥嘉" },
      { title: "早开的晚霞-林宥嘉", artist: "林宥嘉" },
      { title: "晚安-林宥嘉", artist: "林宥嘉" },
      { title: "纪念品-林宥嘉", artist: "林宥嘉" },
      { title: "美妙生活-林宥嘉", artist: "林宥嘉" },
      { title: "自然醒-林宥嘉", artist: "林宥嘉" },
    ],
    expectedAlbum: "美妙生活",
  },
];

// ── Test runner ────────────────────────────────────────────────────

describeLlm("LLM tag correction evaluation", () => {
  let passCount = 0;
  let failCount = 0;
  const results: Array<{ name: string; pass: boolean; expected: string; got: string; confidence: number }> = [];

  beforeAll(() => {
    console.log(`\n  LLM model: ${LLM_MODEL}`);
    console.log(`  Test cases: ${TEST_CASES.length}\n`);
  });

  for (const tc of TEST_CASES) {
    it(tc.name, { timeout: LLM_TIMEOUT }, async () => {
      expect(client).toBeDefined();
      if (!client) return;

      const messages = buildTagCorrectionMessages(
        tc.folderName,
        tc.parentName,
        tc.parsedArtist,
        tc.parsedAlbum,
        tc.parsedYear,
        tc.tracks,
      );

      const response = await client.completeJson(
        messages,
        "TagCorrectionResponse",
        TAG_CORRECTION_SCHEMA,
        undefined,
        { allowMessageFallback: true },
      );

      const data = response.data as Record<string, unknown>;
      const album = (data.album as string) ?? "";
      const confidence = (data.confidence as number) ?? 0;

      // Log full response for debugging
      console.log(`\n    [${tc.name}] Full LLM response:`);
      console.log(`      artist: ${data.artist}`);
      console.log(`      albumArtist: ${data.albumArtist}`);
      console.log(`      album: ${JSON.stringify(data.album)}`);
      console.log(`      year: ${data.year}`);
      console.log(`      genre: ${data.genre}`);
      console.log(`      confidence: ${data.confidence}`);
      if (data.tracks && Array.isArray(data.tracks)) {
        console.log(`      tracks:`);
        for (const t of data.tracks as Array<Record<string, unknown>>) {
          console.log(`        ${t.index}: title=${JSON.stringify(t.title)} artist=${JSON.stringify(t.artist)}`);
        }
      }

      const pass = album === tc.expectedAlbum;
      if (pass) passCount++;
      else failCount++;
      results.push({ name: tc.name, pass, expected: tc.expectedAlbum, got: album, confidence });

      const tag = pass ? "PASS" : "FAIL";
      console.log(
        `  [${tag}] ${tc.name}` +
        `\n    parsed="${tc.parsedAlbum}" → LLM="${album}" (expected="${tc.expectedAlbum}") conf=${confidence}`,
      );

      expect(album).toBe(tc.expectedAlbum);
    });
  }

  it("summary report", () => {
    // This always runs after the eval cases — prints the summary
    console.log(`\n  ═══════════════════════════════════════════════`);
    console.log(`  LLM Tag Correction Eval Summary`);
    console.log(`  Model: ${LLM_MODEL}`);
    console.log(`  Total: ${TEST_CASES.length}  Pass: ${passCount}  Fail: ${failCount}`);
    console.log(`  Pass rate: ${TEST_CASES.length > 0 ? Math.round((passCount / TEST_CASES.length) * 100) : 0}%`);
    console.log(`  ═══════════════════════════════════════════════\n`);

    for (const r of results) {
      const icon = r.pass ? "✓" : "✗";
      console.log(`  ${icon} ${r.name}: "${r.got}" ${r.pass ? "==" : "!="} "${r.expected}"`);
    }
    console.log("");
  });
});
