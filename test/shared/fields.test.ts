// @vitest-environment node
import { describe, it, expect } from "vitest";
import { parseDiscField } from "../../src/shared/fields";

describe("parseDiscField", () => {
  it('parses "1" as discNumber: 1', () => {
    expect(parseDiscField("1")).toEqual({ discNumber: 1 });
  });

  it('parses "1/2" as discNumber: 1, discTotal: 2', () => {
    expect(parseDiscField("1/2")).toEqual({ discNumber: 1, discTotal: 2 });
  });

  it('parses "0" as empty (0 is falsy in parseInt but not nullish)', () => {
    // parseInt("0", 10) === 0, and 0 || undefined → undefined
    expect(parseDiscField("0")).toEqual({});
  });

  it("returns empty for empty string", () => {
    expect(parseDiscField("")).toEqual({});
  });

  it("returns empty for unparseable input", () => {
    expect(parseDiscField("abc")).toEqual({});
  });

  it("ignores extra parts after second slash", () => {
    expect(parseDiscField("1/2/3")).toEqual({ discNumber: 1, discTotal: 2 });
  });
});
