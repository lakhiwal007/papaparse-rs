/// <reference types="node" />
// Types of papaparse-wasm and papaparse-node: the `Papa` API of PapaParse 5.7.0 for strings and
// Node.js streams (no `download`, `File` or web workers).

import type { Duplex, Readable } from 'node:stream';

export type Cell = string | number | boolean | null | Date;
export type Row = Cell[] | Record<string, Cell | Cell[]>;

export interface ParseError {
  type: 'Quotes' | 'Delimiter' | 'FieldMismatch';
  code: 'MissingQuotes' | 'InvalidQuotes' | 'UndetectableDelimiter' | 'TooManyFields' | 'TooFewFields';
  message: string;
  row?: number;
  index?: number;
}

export interface ParseMeta {
  delimiter: string;
  linebreak: string;
  aborted: boolean;
  truncated: boolean;
  cursor: number;
  renamedHeaders: Record<string, string> | null;
  fields?: string[];
  paused?: boolean;
}

export interface ParseResult<T> {
  data: T[];
  errors: ParseError[];
  meta: ParseMeta;
}

/** What a `step` callback receives: one row. */
export interface ParseStepResult<T> {
  data: T;
  errors: ParseError[];
  meta: ParseMeta;
}

/** The handle passed to `step` and `chunk` callbacks. */
export interface Parser {
  abort(): void;
  pause(): void;
  resume(): void;
  paused(): boolean;
  aborted(): boolean;
}

export interface ParseConfig<T = Row> {
  /** A string, or a function of the input returning one; detected when empty. */
  delimiter?: string | ((input: string) => string);
  newline?: '\n' | '\r' | '\r\n';
  quoteChar?: string;
  escapeChar?: string;
  header?: boolean;
  transformHeader?(header: string, index: number): string;
  dynamicTyping?: boolean | Record<string | number, boolean> | ((field: string | number) => boolean);
  preview?: number;
  comments?: boolean | string;
  skipEmptyLines?: boolean | 'greedy';
  skipFirstNLines?: number;
  fastMode?: boolean;
  delimitersToGuess?: string[];
  /** Rewrites each cell before dynamic typing; may return any value. */
  transform?(value: string, field: string | number): unknown;
  /** Parse in pieces of this many UTF-16 code units (strings only). */
  chunkSize?: number;
  /** For Node.js streams that yield Buffers. */
  encoding?: BufferEncoding;
  step?(results: ParseStepResult<T>, parser: Parser): void;
  chunk?(results: ParseResult<T>, parser: Parser): void;
  complete?(results: ParseResult<T>, file?: unknown): void;
  error?(error: Error, file?: unknown): void;
  beforeFirstChunk?(chunk: string): string | void;
  /** Accepted for compatibility; parsing always runs on the calling thread. */
  worker?: boolean;
}

export interface UnparseConfig {
  quotes?: boolean | boolean[] | ((value: unknown, column: number) => boolean);
  quoteChar?: string;
  escapeChar?: string;
  delimiter?: string;
  header?: boolean;
  newline?: string;
  skipEmptyLines?: boolean | 'greedy';
  columns?: string[];
  escapeFormulae?: boolean | RegExp;
}

export type UnparseInput =
  | unknown[][]
  | Record<string, unknown>[]
  | { fields?: string[]; data?: unknown[] | string; meta?: { fields?: string[] } }
  | string;

/** Parses CSV text. Same semantics as `Papa.parse(string, config)`. */
export function parse<T = Record<string, Cell | Cell[]>>(input: string, config: ParseConfig<T> & { header: true }): ParseResult<T>;
export function parse<T = Cell[]>(input: string, config?: ParseConfig<T>): ParseResult<T>;
/** Parses a Node.js readable stream; results arrive through `step` / `chunk` / `complete`. */
export function parse<T = Row>(input: Readable, config: ParseConfig<T>): void;
/** Returns a duplex stream: write CSV text in, read rows out. */
export function parse<T = Row>(input: typeof NODE_STREAM_INPUT, config?: ParseConfig<T>): Duplex;

/** Writes rows as CSV. Same semantics as `Papa.unparse(input, config)`. */
export function unparse(input: UnparseInput, config?: UnparseConfig): string;

export const RECORD_SEP: string;
export const UNIT_SEP: string;
export const BYTE_ORDER_MARK: string;
export const BAD_DELIMITERS: string[];
export const WORKERS_SUPPORTED: false;
export const NODE_STREAM_INPUT: 1;
export let LocalChunkSize: number;
export let RemoteChunkSize: number;
export let DefaultDelimiter: string;
