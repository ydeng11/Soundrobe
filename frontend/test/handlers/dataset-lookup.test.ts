// @vitest-environment node
/**
 * Tests for DatasetReader lookup query — specifically the artist-only fallback
 * guard that prevents unrelated "Various Artists" compilations from being returned.
 *
 * These tests exercise the static helpers without needing better-sqlite3.
 */

import { describe, it, expect } from "vitest";
import { DatasetReader } from "../../electron/handlers/dataset";

describe("DatasetReader — isGenericArtist", () => {
  it("returns true for common generic artist names", () => {
    expect(DatasetReader.isGenericArtist("various artists")).toBe(true);
    expect(DatasetReader.isGenericArtist("Various Artists")).toBe(true);
    expect(DatasetReader.isGenericArtist("VARIous ARTISTS")).toBe(true);
    expect(DatasetReader.isGenericArtist("va")).toBe(true);
    expect(DatasetReader.isGenericArtist("VA")).toBe(true);
    expect(DatasetReader.isGenericArtist("various")).toBe(true);
    expect(DatasetReader.isGenericArtist("unknown artist")).toBe(true);
    expect(DatasetReader.isGenericArtist("unknown")).toBe(true);
  });

  it("returns false for real artist names", () => {
    expect(DatasetReader.isGenericArtist("郑伊健")).toBe(false);
    expect(DatasetReader.isGenericArtist("陈小春")).toBe(false);
    expect(DatasetReader.isGenericArtist("谢天华")).toBe(false);
    expect(DatasetReader.isGenericArtist("周杰伦")).toBe(false);
    expect(DatasetReader.isGenericArtist("Pink Floyd")).toBe(false);
    expect(DatasetReader.isGenericArtist("The Beatles")).toBe(false);
  });

  it("handles whitespace and casing", () => {
    expect(DatasetReader.isGenericArtist("  VarioUs ArtisTs  ")).toBe(true);
    expect(DatasetReader.isGenericArtist("\tvarious artists\n")).toBe(true);
  });

  it("returns false for empty or blank strings", () => {
    expect(DatasetReader.isGenericArtist("")).toBe(false);
    expect(DatasetReader.isGenericArtist("   ")).toBe(false);
  });
});
