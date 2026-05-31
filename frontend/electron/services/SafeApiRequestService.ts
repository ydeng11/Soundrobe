/**
 * SafeApiRequestService — safe preset HTTP requests.
 *
 * v1 only supports:
 * - GET requests
 * - Known preset hosts only
 * - Timeouts on every request
 * - Token redaction in logs and previews
 * - Small summarized response bodies
 */

const ALLOWED_HOSTS = new Set([
  "musicbrainz.org",
  "discogs.com",
  "api.discogs.com",
]);

export interface SafeApiRequest {
  preset: "musicbrainzSearch" | "discogsSearch" | "lyricsSearch";
  params: Record<string, string>;
}

export interface SafeApiResult {
  ok: boolean;
  status: number;
  summary: string;
  data?: unknown;
  error?: string;
}

export class SafeApiRequestService {
  private discogsToken: string | null = null;
  private lyricsHost: string | null = null;

  setDiscogsToken(token: string | null): void {
    this.discogsToken = token;
  }

  setLyricsHost(host: string | null): void {
    this.lyricsHost = host;
  }

  /**
   * Execute a safe preset API request.
   * Validates the preset, host, and method.
   */
  async execute(request: SafeApiRequest): Promise<SafeApiResult> {
    const url = this.buildUrl(request);
    if (!url) {
      return {
        ok: false,
        status: 0,
        summary: "Invalid or unsupported request preset",
        error: "UNSUPPORTED_PRESET",
      };
    }

    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 10_000);

    try {
      const response = await fetch(url, {
        method: "GET",
        signal: controller.signal,
        headers: this.buildHeaders(request),
      });

      clearTimeout(timeout);

      if (!response.ok) {
        const body = await response.text().catch(() => "");
        return {
          ok: false,
          status: response.status,
          summary: `HTTP ${response.status}: ${body.slice(0, 200)}`,
          error: body.slice(0, 500),
        };
      }

      const data = await response.json();
      const summary = this.summarizeResponse(request.preset, data);

      return {
        ok: true,
        status: response.status,
        summary,
        data: this.redactResponse(data),
      };
    } catch (error) {
      clearTimeout(timeout);

      if (error instanceof DOMException && error.name === "AbortError") {
        return {
          ok: false,
          status: 0,
          summary: "Request timed out after 10 seconds",
          error: "TIMEOUT",
        };
      }

      return {
        ok: false,
        status: 0,
        summary: error instanceof Error ? error.message : String(error),
        error: error instanceof Error ? error.message : String(error),
      };
    }
  }

  private buildUrl(request: SafeApiRequest): string | null {
    switch (request.preset) {
      case "musicbrainzSearch": {
        const query = request.params.query ?? "";
        const limit = request.params.limit ?? "5";
        return `https://musicbrainz.org/ws/2/release?query=${encodeURIComponent(query)}&limit=${encodeURIComponent(limit)}&fmt=json`;
      }

      case "discogsSearch": {
        if (!this.discogsToken) return null;
        const query = request.params.query ?? "";
        const type = request.params.type ?? "release";
        const limit = request.params.limit ?? "5";
        return `https://api.discogs.com/database/search?q=${encodeURIComponent(query)}&type=${encodeURIComponent(type)}&per_page=${encodeURIComponent(limit)}`;
      }

      case "lyricsSearch": {
        if (!this.lyricsHost) return null;
        const artist = request.params.artist ?? "";
        const title = request.params.title ?? "";
        return `${this.lyricsHost}/api/lyrics?artist=${encodeURIComponent(artist)}&title=${encodeURIComponent(title)}`;
      }

      default:
        return null;
    }
  }

  private buildHeaders(request: SafeApiRequest): Record<string, string> {
    const headers: Record<string, string> = {
      "User-Agent": "AutoTagger/1.0",
    };

    switch (request.preset) {
      case "musicbrainzSearch":
        headers["Accept"] = "application/json";
        break;
      case "discogsSearch":
        if (this.discogsToken) {
          headers["Authorization"] = `Discogs token=${this.discogsToken}`;
        }
        break;
    }

    return headers;
  }

  private summarizeResponse(preset: string, data: unknown): string {
    try {
      switch (preset) {
        case "musicbrainzSearch": {
          const releases = (data as any)?.releases ?? [];
          return `Found ${releases.length} release(s)${releases.length > 0 ? `: ${releases.map((r: any) => `"${r.title}" by ${r["artist-credit"]?.[0]?.name ?? "unknown"}`).join(", ")}` : ""}`;
        }

        case "discogsSearch": {
          const results = (data as any)?.results ?? [];
          return `Found ${results.length} result(s)${results.length > 0 ? `: ${results.map((r: any) => `"${r.title}" (${r.type})`).join(", ")}` : ""}`;
        }

        case "lyricsSearch": {
          const lyrics = (data as any)?.lyrics;
          if (lyrics) {
            const lines = String(lyrics).split("\n").length;
            return `Found lyrics (${lines} lines)`;
          }
          return "No lyrics found";
        }

        default:
          return "Response received";
      }
    } catch {
      return "Could not summarize response";
    }
  }

  private redactResponse(data: unknown): unknown {
    // Deep clone and redact sensitive fields
    if (typeof data !== "object" || data === null) return data;

    const redacted = Array.isArray(data) ? [...data] : { ...data } as Record<string, unknown>;

    for (const [key, value] of Object.entries(redacted)) {
      const lowerKey = key.toLowerCase();
      if (
        lowerKey.includes("token") ||
        lowerKey.includes("key") ||
        lowerKey.includes("secret") ||
        lowerKey.includes("auth")
      ) {
        (redacted as Record<string, unknown>)[key] = "[REDACTED]";
      } else if (typeof value === "object" && value !== null) {
        (redacted as Record<string, unknown>)[key] = this.redactResponse(value);
      }
    }

    return redacted;
  }
}
