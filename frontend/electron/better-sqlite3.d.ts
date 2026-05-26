/** Minimal type declarations for better-sqlite3 used by Auto Tagger handlers. */
declare module "better-sqlite3" {
  interface Statement {
    run(...params: unknown[]): { changes: number; lastInsertRowid: number | bigint };
    get<T = Record<string, unknown>>(...params: unknown[]): T | undefined;
    all<T = Record<string, unknown>>(...params: unknown[]): T[];
    iterate<T = Record<string, unknown>>(...params: unknown[]): IterableIterator<T>;
    columns(): { name: string }[];
    readonly reader: boolean;
    readonly source: string;
    readonly database: Database;
  }

  interface DatabaseOptions {
    readonly?: boolean;
    memory?: boolean;
    nativeBinding?: string;
    timeout?: number;
  }

  class Database {
    constructor(filename: string, options?: DatabaseOptions);
    prepare(sql: string): Statement;
    exec(sql: string): void;
    pragma(sql: string): unknown;
    close(): void;
    readonly memory: boolean;
    readonly readonly: boolean;
    readonly name: string;
    readonly open: boolean;
  }

  export default Database;
}
