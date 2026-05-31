/**
 * AssistantToolRegistry — fixed registry of assistant tools.
 *
 * Every tool has:
 * - name: unique identifier
 * - description: what it does (shown to the LLM)
 * - inputSchema: JSON Schema for argument validation
 * - executor: async function that processes the call
 * - isReadOnly: true for safe, non-mutating tools
 * - riskLevel: "low" | "medium" | "high" for mutating tools
 */

export type ToolExecutor = (
  args: Record<string, unknown>,
) => Promise<AssistantToolResult>;

export interface AssistantToolDef {
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;
  executor: ToolExecutor;
  isReadOnly: boolean;
  riskLevel?: "low" | "medium" | "high";
}

export interface AssistantToolResult {
  ok: boolean;
  summary: string;
  data?: unknown;
  pendingActionBatchId?: string;
  error?: string;
}

export class AssistantToolRegistry {
  private tools = new Map<string, AssistantToolDef>();

  /**
   * Register a tool. Throws on duplicate name.
   */
  register(tool: AssistantToolDef): void {
    if (this.tools.has(tool.name)) {
      throw new Error(`Duplicate tool name: ${tool.name}`);
    }
    this.tools.set(tool.name, tool);
  }

  /**
   * Bulk register tools.
   */
  registerAll(tools: AssistantToolDef[]): void {
    for (const tool of tools) {
      this.register(tool);
    }
  }

  /**
   * Get a tool by name. Returns undefined for unknown tools.
   */
  get(name: string): AssistantToolDef | undefined {
    return this.tools.get(name);
  }

  /**
   * Get all registered tools (for passing to the LLM).
   */
  getAll(): AssistantToolDef[] {
    return Array.from(this.tools.values());
  }

  /**
   * Get only read-only tools.
   */
  getReadOnly(): AssistantToolDef[] {
    return this.getAll().filter((t) => t.isReadOnly);
  }

  /**
   * Get only mutating tools.
   */
  getMutating(): AssistantToolDef[] {
    return this.getAll().filter((t) => !t.isReadOnly);
  }

  /**
   * Execute a tool by name with validated args.
   * Returns error result for unknown tool or invalid args.
   */
  async execute(
    toolName: string,
    args: Record<string, unknown>,
  ): Promise<AssistantToolResult> {
    const tool = this.tools.get(toolName);
    if (!tool) {
      return {
        ok: false,
        summary: `Unknown tool: ${toolName}`,
        error: `Tool "${toolName}" is not in the registry`,
      };
    }

    // Basic type validation against schema
    const validationError = this.validateArgs(tool.inputSchema, args);
    if (validationError) {
      return {
        ok: false,
        summary: `Invalid arguments for ${toolName}: ${validationError}`,
        error: validationError,
      };
    }

    try {
      return await tool.executor(args);
    } catch (error) {
      return {
        ok: false,
        summary: `Tool ${toolName} failed: ${error instanceof Error ? error.message : String(error)}`,
        error: error instanceof Error ? error.message : String(error),
      };
    }
  }

  /**
   * Small JSON Schema validator for assistant tool arguments.
   *
   * This deliberately supports only the schema features used by local tools:
   * object properties, required fields, primitive types, arrays, and enums.
   */
  private validateArgs(
    schema: Record<string, unknown>,
    args: Record<string, unknown>,
  ): string | null {
    const properties = (schema.properties as Record<string, unknown>) ?? {};
    const required = (schema.required as string[]) ?? [];

    // Check required fields
    for (const field of required) {
      if (args[field] === undefined || args[field] === null) {
        return `Missing required field: ${field}`;
      }
    }

    // Reject unknown fields so the assistant does not silently hallucinate knobs.
    for (const [key, value] of Object.entries(args)) {
      const propSchema = properties[key] as Record<string, unknown> | undefined;
      if (!propSchema) {
        return `Unknown field: ${key}`;
      }

      if (value === undefined || value === null) continue;

      const validationError = this.validateValue(key, propSchema, value);
      if (validationError) return validationError;
    }

    return null;
  }

  private validateValue(
    fieldPath: string,
    schema: Record<string, unknown>,
    value: unknown,
  ): string | null {
    const expectedType = schema.type as string | undefined;
    const actualType = Array.isArray(value) ? "array" : typeof value;

    if (expectedType && actualType !== expectedType) {
      return `Field "${fieldPath}" should be a ${expectedType}, got ${actualType}`;
    }

    const enumValues = schema.enum as unknown[] | undefined;
    if (enumValues && !enumValues.includes(value)) {
      return `Field "${fieldPath}" should be one of: ${enumValues.join(", ")}`;
    }

    if (expectedType === "array" && Array.isArray(value)) {
      const itemSchema = schema.items as Record<string, unknown> | undefined;
      if (!itemSchema) return null;

      for (let i = 0; i < value.length; i++) {
        const validationError = this.validateValue(
          `${fieldPath}[${i}]`,
          itemSchema,
          value[i],
        );
        if (validationError) return validationError;
      }
    }

    if (expectedType === "object" && value && typeof value === "object" && !Array.isArray(value)) {
      const nestedSchema = schema as Record<string, unknown>;
      return this.validateArgs(nestedSchema, value as Record<string, unknown>);
    }

    return null;
  }
}
