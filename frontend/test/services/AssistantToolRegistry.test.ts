import { describe, it, expect, vi } from "vitest";
import { AssistantToolRegistry } from "../../electron/services/AssistantToolRegistry";
import type { AssistantToolDef } from "../../electron/services/AssistantToolRegistry";

describe("AssistantToolRegistry", () => {
  function makeTool(name: string, overrides: Partial<AssistantToolDef> = {}): AssistantToolDef {
    return {
      name,
      description: `Tool ${name}`,
      inputSchema: { type: "object", properties: {}, required: [] },
      executor: vi.fn().mockResolvedValue({ ok: true, summary: "ok" }),
      isReadOnly: true,
      ...overrides,
    };
  }

  describe("register", () => {
    it("registers a tool", () => {
      const reg = new AssistantToolRegistry();
      reg.register(makeTool("test.tool"));
      expect(reg.get("test.tool")).toBeDefined();
    });

    it("rejects duplicate names", () => {
      const reg = new AssistantToolRegistry();
      reg.register(makeTool("dup"));
      expect(() => reg.register(makeTool("dup"))).toThrow("Duplicate tool name: dup");
    });
  });

  describe("registerAll", () => {
    it("registers multiple tools", () => {
      const reg = new AssistantToolRegistry();
      reg.registerAll([makeTool("a"), makeTool("b"), makeTool("c")]);
      expect(reg.getAll()).toHaveLength(3);
    });
  });

  describe("get", () => {
    it("returns undefined for unknown tools", () => {
      const reg = new AssistantToolRegistry();
      expect(reg.get("nonexistent")).toBeUndefined();
    });
  });

  describe("getReadOnly / getMutating", () => {
    it("classifies tools correctly", () => {
      const reg = new AssistantToolRegistry();
      reg.register(makeTool("read.only", { isReadOnly: true }));
      reg.register(makeTool("mutating.one", { isReadOnly: false }));

      expect(reg.getReadOnly()).toHaveLength(1);
      expect(reg.getMutating()).toHaveLength(1);
      expect(reg.getReadOnly()[0].name).toBe("read.only");
      expect(reg.getMutating()[0].name).toBe("mutating.one");
    });
  });

  describe("execute", () => {
    it("executes known tools with valid args", async () => {
      const executor = vi.fn().mockResolvedValue({ ok: true, summary: "done" });
      const reg = new AssistantToolRegistry();
      reg.register(makeTool("test.exec", { executor, isReadOnly: true }));

      const result = await reg.execute("test.exec", {});
      expect(result.ok).toBe(true);
      expect(executor).toHaveBeenCalledWith({});
    });

    it("returns error for unknown tools", async () => {
      const reg = new AssistantToolRegistry();
      const result = await reg.execute("nonexistent", {});
      expect(result.ok).toBe(false);
      expect(result.error).toContain("not in the registry");
    });

    it("validates required fields", async () => {
      const reg = new AssistantToolRegistry();
      reg.register(makeTool("needs.arg", {
        isReadOnly: true,
        inputSchema: {
          type: "object",
          properties: { name: { type: "string" } },
          required: ["name"],
        },
      }));

      const result = await reg.execute("needs.arg", {});
      expect(result.ok).toBe(false);
      expect(result.error).toContain("Missing required field: name");
    });

    it("rejects unknown fields", async () => {
      const reg = new AssistantToolRegistry();
      reg.register(makeTool("known.args", {
        inputSchema: {
          type: "object",
          properties: { name: { type: "string" } },
          required: [],
        },
      }));

      const result = await reg.execute("known.args", { name: "ok", extra: "nope" });
      expect(result.ok).toBe(false);
      expect(result.error).toContain("Unknown field: extra");
    });

    it("validates enum values", async () => {
      const reg = new AssistantToolRegistry();
      reg.register(makeTool("enum.arg", {
        inputSchema: {
          type: "object",
          properties: {
            criterion: {
              type: "string",
              enum: ["extension", "pattern"],
            },
          },
          required: ["criterion"],
        },
      }));

      const result = await reg.execute("enum.arg", { criterion: "album" });
      expect(result.ok).toBe(false);
      expect(result.error).toContain("should be one of: extension, pattern");
    });

    it("validates array item types", async () => {
      const reg = new AssistantToolRegistry();
      reg.register(makeTool("array.arg", {
        inputSchema: {
          type: "object",
          properties: {
            paths: {
              type: "array",
              items: { type: "string" },
            },
          },
          required: ["paths"],
        },
      }));

      const result = await reg.execute("array.arg", { paths: ["/ok.flac", 12] });
      expect(result.ok).toBe(false);
      expect(result.error).toContain('Field "paths[1]" should be a string');
    });

    it("catches executor errors", async () => {
      const reg = new AssistantToolRegistry();
      reg.register(makeTool("err", {
        isReadOnly: true,
        executor: vi.fn().mockRejectedValue(new Error("boom")),
      }));

      const result = await reg.execute("err", {});
      expect(result.ok).toBe(false);
      expect(result.error).toContain("boom");
    });
  });
});
