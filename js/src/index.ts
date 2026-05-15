/**
 * json-slabs — efficient serialization + deserialization of "JSON with binary
 * slabs" for JavaScript objects which contain typed arrays.
 *
 * `encode(obj)` returns a `Uint8Array` with the JSLB file bytes. Typed arrays
 * anywhere in the object's structure are embedded verbatim within the returned
 * buffer as "slabs" whose location is indicated in the file header; their
 * positions in the JSON are replaced by `{ "$s": N }` placeholders.
 *
 * `decode(buffer)` recreates the original object structure, with typed arrays
 * restored in the right spots as zero-copy views into the buffer's underlying
 * `ArrayBuffer`.
 *
 * High-level API:
 *   encode(obj, splitOut?)        — object -> Uint8Array with the JSLB file bytes
 *   encodeToBlob(obj, splitOut?)  — object -> Blob (avoids the large allocation / copy that `encode` performs; suitable for streams)
 *   decode<T>(buffer)             — Uint8Array -> T
 *   isJsonSlabsFile(buffer)       - Uint8Array -> boolean
 *
 * Low-level API:
 *   new Builder()                 — low-level builder for manual slab construction
 *   decodeContainer(buffer)       — low-level reader returning the raw slab table
 *
 * `splitOut` is an optional list of nested values (matched by reference
 * identity) that each get lifted out of the root JSON into their own
 * SlabType.Json slab.
 */

// 0xDC 0xDF: invalid UTF-8, high bits set. "JSLB": ASCII name. 0x01 0x00: LE endianness marker.
const MAGIC = new Uint8Array([0xdc, 0xdf, 0x4a, 0x53, 0x4c, 0x42, 0x01, 0x00]);
const VERSION = 1;

const FIXED_HEADER_SIZE = 20; // magic(8) + version(4) + slabCount(4) + rootIndex(4)
const SLAB_TABLE_ENTRY_SIZE = 12; // type(4) + startOffset(4) + byteLength(4)

export const SlabType = {
  Int8: 0x00,
  Uint8: 0x01,
  Int16: 0x02,
  Uint16: 0x03,
  Int32: 0x04,
  Uint32: 0x05,
  Float32: 0x06,
  Float64: 0x07,
  Int64: 0x08,
  Uint64: 0x09,
  Json: 0x0a,
} as const;
export type SlabType = (typeof SlabType)[keyof typeof SlabType];

function alignUp(pos: number, alignment: number): number {
  return (pos + alignment - 1) & ~(alignment - 1);
}

function elementSizeForTypeByte(typeByte: number): number {
  switch (typeByte) {
    case SlabType.Int16:
    case SlabType.Uint16:
      return 2;
    case SlabType.Int32:
    case SlabType.Uint32:
    case SlabType.Float32:
      return 4;
    case SlabType.Float64:
    case SlabType.Int64:
    case SlabType.Uint64:
      return 8;
    default:
      return 1;
  }
}

export type AnySlab =
  | Int8Array
  | Uint8Array
  | Int16Array
  | Uint16Array
  | Int32Array
  | Uint32Array
  | Float32Array
  | Float64Array
  | BigInt64Array
  | BigUint64Array;

export type TaggedSlab =
  | { type: typeof SlabType.Int8; array: Int8Array }
  | { type: typeof SlabType.Uint8; array: Uint8Array }
  | { type: typeof SlabType.Int16; array: Int16Array }
  | { type: typeof SlabType.Uint16; array: Uint16Array }
  | { type: typeof SlabType.Int32; array: Int32Array }
  | { type: typeof SlabType.Uint32; array: Uint32Array }
  | { type: typeof SlabType.Float32; array: Float32Array }
  | { type: typeof SlabType.Float64; array: Float64Array }
  | { type: typeof SlabType.Int64; array: BigInt64Array }
  | { type: typeof SlabType.Uint64; array: BigUint64Array }
  | { type: typeof SlabType.Json; jsonBytes: Uint8Array };

export type SlabPlaceholder = { $s: number };

export type DecodedContainer = {
  // All slabs in order, as zero-copy views into the input buffer.
  slabs: TaggedSlab[];
  rootJsonSlabIndex: number;
  rootJsonBytes: Uint8Array;
};

export class Builder {
  private readonly _entries: Array<{
    typeByte: SlabType;
    view: ArrayBufferView;
  }> = [];
  private _finished = false;

  /**
   * Register a TypedArray as a binary slab and return a `{ "$s": N }`
   * placeholder to embed in the JSON skeleton.
   */
  addSlab(slab: AnySlab | Uint8ClampedArray): SlabPlaceholder {
    this._checkNotFinished();
    if (slab instanceof Int8Array) return this._push(SlabType.Int8, slab);
    if (slab instanceof Uint8Array || slab instanceof Uint8ClampedArray)
      return this._push(SlabType.Uint8, slab);
    if (slab instanceof Int16Array) return this._push(SlabType.Int16, slab);
    if (slab instanceof Uint16Array) return this._push(SlabType.Uint16, slab);
    if (slab instanceof Int32Array) return this._push(SlabType.Int32, slab);
    if (slab instanceof Uint32Array) return this._push(SlabType.Uint32, slab);
    if (slab instanceof Float32Array) return this._push(SlabType.Float32, slab);
    if (slab instanceof Float64Array) return this._push(SlabType.Float64, slab);
    if (slab instanceof BigInt64Array) return this._push(SlabType.Int64, slab);
    if (slab instanceof BigUint64Array)
      return this._push(SlabType.Uint64, slab);
    throw new TypeError('Unsupported TypedArray');
  }

  /** Register a nested JSON document as a slab. Will be UTF-8 encoded if a string. */
  addJsonSlab(json: string | Uint8Array): SlabPlaceholder {
    this._checkNotFinished();
    const bytes =
      typeof json === 'string' ? new TextEncoder().encode(json) : json;
    return this._push(SlabType.Json, bytes);
  }

  private _push(typeByte: SlabType, view: ArrayBufferView): SlabPlaceholder {
    const bin = this._entries.length;
    this._entries.push({ typeByte, view });
    return { $s: bin };
  }

  private _checkNotFinished(): void {
    if (this._finished) throw new Error('Builder already finished');
  }

  /**
   * Appends the JSON slab and returns the container as a list of chunks.
   * Slab data is returned as zero-copy views; only the header is newly
   * allocated. Callers can stream the chunks or concatenate as needed.
   * The Builder must not be used after this call.
   */
  finish(json: string | Uint8Array): Uint8Array[] {
    this._checkNotFinished();
    this._finished = true;
    const jsonBytes =
      typeof json === 'string' ? new TextEncoder().encode(json) : json;
    const rootJsonSlabIndex = this._entries.length;
    this._entries.push({ typeByte: SlabType.Json, view: jsonBytes });

    const slabCount = this._entries.length;
    const slabTableEnd = FIXED_HEADER_SIZE + slabCount * SLAB_TABLE_ENTRY_SIZE;

    // Compute the absolute start offset of each slab, aligned to its element size.
    const startOffsets: number[] = [];
    let pos = slabTableEnd;
    for (const { typeByte, view } of this._entries) {
      pos = alignUp(pos, elementSizeForTypeByte(typeByte));
      startOffsets.push(pos);
      pos += view.byteLength;
    }

    const header = new Uint8Array(startOffsets[0]!);
    const dv = new DataView(header.buffer);

    header.set(MAGIC, 0);
    dv.setUint32(8, VERSION, true);
    dv.setUint32(12, slabCount, true);
    dv.setUint32(16, rootJsonSlabIndex, true);

    let tablePos = FIXED_HEADER_SIZE;
    for (let i = 0; i < slabCount; i++) {
      const { typeByte, view } = this._entries[i]!;
      dv.setUint32(tablePos, typeByte, true);
      dv.setUint32(tablePos + 4, startOffsets[i]!, true);
      dv.setUint32(tablePos + 8, view.byteLength, true);
      tablePos += SLAB_TABLE_ENTRY_SIZE;
    }

    // Emit header, then slabs with alignment padding between them.
    const chunks: Uint8Array[] = [header];
    const zeroPad = new Uint8Array(8); // max alignment is 8; reused for all gaps
    let dataPos = startOffsets[0]!;

    for (let i = 0; i < slabCount; i++) {
      const startOff = startOffsets[i]!;
      if (startOff > dataPos)
        chunks.push(zeroPad.subarray(0, startOff - dataPos));
      const { view } = this._entries[i]!;
      chunks.push(
        new Uint8Array(view.buffer, view.byteOffset, view.byteLength),
      );
      dataPos = startOff + view.byteLength;
    }

    return chunks;
  }

  /** Finish and return the container as a single contiguous `Uint8Array`. */
  toBuffer(json: string | Uint8Array): Uint8Array<ArrayBuffer> {
    this._checkNotFinished();
    const chunks = this.finish(json);
    const total = chunks.reduce((s, c) => s + c.byteLength, 0);
    const out = new Uint8Array(total);
    let off = 0;
    for (const c of chunks) {
      out.set(c, off);
      off += c.byteLength;
    }
    return out as Uint8Array<ArrayBuffer>;
  }

  /** Finish and return the container as a `Blob` (zero-copy from chunks). */
  toBlob(json: string | Uint8Array): Blob {
    this._checkNotFinished();
    return new Blob(this.finish(json) as BlobPart[]);
  }
}

export function isJsonSlabsFile(buffer: Uint8Array): boolean {
  if (buffer.byteLength < MAGIC.length) return false;
  for (let i = 0; i < MAGIC.length; i++) {
    if (buffer[i] !== MAGIC[i]) return false;
  }
  return true;
}

export function decodeContainer(buffer: Uint8Array): DecodedContainer {
  if (buffer.byteLength < FIXED_HEADER_SIZE) {
    throw new Error(
      `Not a JSLB container: buffer too short (${buffer.byteLength} < ${FIXED_HEADER_SIZE} bytes)`,
    );
  }

  for (let i = 0; i < MAGIC.length; i++) {
    if (buffer[i] !== MAGIC[i]) {
      throw new Error('Not a JSLB container: bad magic bytes');
    }
  }

  // Zero-copy decoding requires the container to start at an 8-byte-aligned
  // offset within its underlying ArrayBuffer, since slab offsets in the
  // header are relative to the container's byte 0.
  if (buffer.byteOffset % 8 !== 0) {
    throw new Error(
      `JSLB container must start at an 8-byte-aligned offset within its underlying ArrayBuffer (got byteOffset=${buffer.byteOffset}). Copy to a fresh Uint8Array before parsing.`,
    );
  }

  const view = new DataView(
    buffer.buffer,
    buffer.byteOffset,
    buffer.byteLength,
  );
  const version = view.getUint32(8, true);
  if (version !== VERSION) {
    throw new Error(`Unsupported JSLB version ${version}`);
  }

  const slabCount = view.getUint32(12, true);
  const rootJsonSlabIndex = view.getUint32(16, true);

  const slabTableEnd = FIXED_HEADER_SIZE + slabCount * SLAB_TABLE_ENTRY_SIZE;
  if (slabTableEnd > buffer.byteLength) {
    throw new Error(
      `JSLB slab table overruns buffer: slabCount=${slabCount} requires ${slabTableEnd} bytes, have ${buffer.byteLength}`,
    );
  }
  if (rootJsonSlabIndex >= slabCount) {
    throw new Error(
      `JSLB rootJsonSlabIndex=${rootJsonSlabIndex} out of range (slabCount=${slabCount})`,
    );
  }

  const slabs = new Array<TaggedSlab>(slabCount);

  let tablePos = FIXED_HEADER_SIZE;
  for (let i = 0; i < slabCount; i++) {
    const typeByte = view.getUint32(tablePos, true);
    const startOff = view.getUint32(tablePos + 4, true);
    const byteLen = view.getUint32(tablePos + 8, true);
    tablePos += SLAB_TABLE_ENTRY_SIZE;

    if (startOff + byteLen > buffer.byteLength) {
      throw new Error(
        `JSLB slab ${i} overruns buffer: startOffset=${startOff} + byteLength=${byteLen} > ${buffer.byteLength}`,
      );
    }
    const elementSize = elementSizeForTypeByte(typeByte);
    if (byteLen % elementSize !== 0) {
      throw new Error(
        `JSLB slab ${i} byteLength=${byteLen} not a multiple of element size ${elementSize} for type ${typeByte}`,
      );
    }

    const absOffset = buffer.byteOffset + startOff;

    slabs[i] = slabView(buffer.buffer, absOffset, typeByte, byteLen);
  }

  const rootJsonSlab = slabs[rootJsonSlabIndex];
  if (rootJsonSlab.type !== SlabType.Json) {
    throw new Error(
      `JSLB rootJsonSlabIndex=${rootJsonSlabIndex} points to slab of type ${rootJsonSlab.type}, expected SlabType.Json (${SlabType.Json})`,
    );
  }

  return {
    slabs,
    rootJsonSlabIndex,
    rootJsonBytes: rootJsonSlab.jsonBytes,
  };
}

function slabView(
  ab: ArrayBufferLike,
  offset: number,
  typeByte: number,
  byteLength: number,
): TaggedSlab {
  switch (typeByte) {
    case SlabType.Uint8:
      return {
        type: SlabType.Uint8,
        array: new Uint8Array(ab, offset, byteLength),
      };
    case SlabType.Int8:
      return {
        type: SlabType.Int8,
        array: new Int8Array(ab, offset, byteLength),
      };
    case SlabType.Int16:
      return {
        type: SlabType.Int16,
        array: new Int16Array(ab, offset, byteLength / 2),
      };
    case SlabType.Uint16:
      return {
        type: SlabType.Uint16,
        array: new Uint16Array(ab, offset, byteLength / 2),
      };
    case SlabType.Int32:
      return {
        type: SlabType.Int32,
        array: new Int32Array(ab, offset, byteLength / 4),
      };
    case SlabType.Uint32:
      return {
        type: SlabType.Uint32,
        array: new Uint32Array(ab, offset, byteLength / 4),
      };
    case SlabType.Float32:
      return {
        type: SlabType.Float32,
        array: new Float32Array(ab, offset, byteLength / 4),
      };
    case SlabType.Float64:
      return {
        type: SlabType.Float64,
        array: new Float64Array(ab, offset, byteLength / 8),
      };
    case SlabType.Int64:
      return {
        type: SlabType.Int64,
        array: new BigInt64Array(ab, offset, byteLength / 8),
      };
    case SlabType.Uint64:
      return {
        type: SlabType.Uint64,
        array: new BigUint64Array(ab, offset, byteLength / 8),
      };
    case SlabType.Json:
      return {
        type: SlabType.Json,
        jsonBytes: new Uint8Array(ab, offset, byteLength),
      };
    default:
      throw new Error('Unknown slab type byte');
  }
}

function stringifyWithSlabs(
  obj: unknown,
  builder: Builder,
  splitOut?: ReadonlyArray<unknown>,
): string {
  const splitSet =
    splitOut && splitOut.length > 0 ? new Set<unknown>(splitOut) : null;
  // Walk the tree once to extract typed-array / split-out values into the
  // builder, and return a copy with `{ "$s": N }` placeholders in their place.
  // Originally we were using JSON.stringify with a replacer here, but it was
  // much slower, especially on long arrays of primitive values.
  const rewritten = rewriteWithPlaceholders(obj, builder, splitSet, true);
  return JSON.stringify(rewritten);
}

// Returns a tree mirroring `value` where typed arrays and `splitOut` targets
// are replaced by `{ "$s": N }` placeholders. Nodes whose descendants need no
// replacement are returned by reference rather than copied.
function rewriteWithPlaceholders(
  value: unknown,
  builder: Builder,
  splitSet: ReadonlySet<unknown> | null,
  isTop: boolean,
): unknown {
  if (value === null || typeof value !== 'object') return value;
  if (ArrayBuffer.isView(value) && !(value instanceof DataView)) {
    return builder.addSlab(value as AnySlab | Uint8ClampedArray);
  }
  // A value passed in `splitOut` that happens to also be the root is not
  // split — the root JSON must not itself be a placeholder.
  if (!isTop && splitSet !== null && splitSet.has(value)) {
    const sub = rewriteContainerWithPlaceholders(value, builder, splitSet);
    return builder.addJsonSlab(JSON.stringify(sub));
  }
  return rewriteContainerWithPlaceholders(value, builder, splitSet);
}

function rewriteContainerWithPlaceholders(
  value: object,
  builder: Builder,
  splitSet: ReadonlySet<unknown> | null,
): unknown {
  if (Array.isArray(value)) {
    return rewriteArrayWithPlaceholders(value, builder, splitSet);
  }
  return rewriteObjectWithPlaceholders(
    value as Record<string, unknown>,
    builder,
    splitSet,
  );
}

function rewriteArrayWithPlaceholders(
  arr: readonly unknown[],
  builder: Builder,
  splitSet: ReadonlySet<unknown> | null,
): unknown[] {
  let result: unknown[] | null = null;
  for (let i = 0; i < arr.length; i++) {
    const el = arr[i];
    // Inline fast-path for primitives: an array of 1000 numbers walks this
    // loop without any function call.
    if (el === null || typeof el !== 'object') continue;
    const replaced = rewriteWithPlaceholders(el, builder, splitSet, false);
    if (replaced !== el) {
      if (result === null) result = arr.slice();
      result[i] = replaced;
    }
  }
  return result ?? (arr as unknown[]);
}

function rewriteObjectWithPlaceholders(
  obj: Record<string, unknown>,
  builder: Builder,
  splitSet: ReadonlySet<unknown> | null,
): Record<string, unknown> {
  let result: Record<string, unknown> | null = null;
  for (const key in obj) {
    if (!Object.hasOwn(obj, key)) continue;
    const el = obj[key];
    if (el === null || typeof el !== 'object') continue;
    const replaced = rewriteWithPlaceholders(el, builder, splitSet, false);
    if (replaced !== el) {
      if (result === null) result = { ...obj };
      result[key] = replaced;
    }
  }
  return result ?? obj;
}

/**
 * Serialize an object (including any nested objects / arrays / typed
 * arrays inside it) to JSLB file bytes.
 *
 * splitOut is an optional array of values which, when encountered as
 * nested values somewhere within the encoded object, will be lifted
 * out of the root JSON into their own `SlabType.Json` slab. Decoding
 * will put them back in the original place.
 *
 * splitOut matching is done by reference identity (`===`).
 */
export function encode(
  obj: unknown,
  splitOut?: ReadonlyArray<unknown>,
): Uint8Array<ArrayBuffer> {
  const builder = new Builder();
  return builder.toBuffer(stringifyWithSlabs(obj, builder, splitOut));
}

/**
 * Serialize an object to a Blob. This is similar to `encode` but saves
 * allocations; rather than building a single buffer with the entire file
 * contents, this builds a Blob from a list of buffers, reusing the original
 * buffers for any typed arrays found in the object graph.
 *
 * Suitable for piping through a CompressionStream.
 *
 * See `encode` for the meaning of `splitOut`.
 */
export function encodeToBlob(
  obj: unknown,
  splitOut?: ReadonlyArray<unknown>,
): Blob {
  const builder = new Builder();
  return builder.toBlob(stringifyWithSlabs(obj, builder, splitOut));
}

/**
 * Deserialize JSLB file bytes back to an object.
 *
 * The optional type parameter `T` lets callers express the expected shape
 * without a separate cast: `decode<MyType>(blob)`. No type validation is
 * performed; use `decode<unknown>` with manual validation if desired.
 */
export function decode<T = unknown>(buffer: Uint8Array): T {
  const { slabs, rootJsonBytes } = decodeContainer(buffer);
  const decoder = new TextDecoder();
  // Do a plain JSON.parse and then walk the tree to expand placeholder
  // objects in a second pass.
  // Originally we were using JSON.parse with a reviver here, but it was
  // much slower, especially on long arrays of primitive values.
  const root = JSON.parse(decoder.decode(rootJsonBytes));
  return withPlaceholdersExpanded(root, slabs) as T;
}

function withPlaceholdersExpanded(
  value: unknown,
  slabs: TaggedSlab[],
): unknown {
  if (value === null || typeof value !== 'object') {
    return value;
  }
  expandPlaceholdersOrReturnSlabIndex(value, slabs);
  return value;
}

function resolveSlab(slabs: TaggedSlab[], slabIndex: number): unknown {
  const slab = slabs[slabIndex];
  if (slab.type !== SlabType.Json) {
    return slab.array;
  }
  const decoder = new TextDecoder();
  const obj = JSON.parse(decoder.decode(slab.jsonBytes));
  return withPlaceholdersExpanded(obj, slabs);
}

function expandPlaceholdersInArray(arr: unknown[], slabs: TaggedSlab[]) {
  for (let i = 0; i < arr.length; i++) {
    const el = arr[i];
    if (typeof el === 'object' && el !== null) {
      const slabIndex = expandPlaceholdersOrReturnSlabIndex(el, slabs);
      if (slabIndex !== -1) {
        arr[i] = resolveSlab(slabs, slabIndex);
      }
    }
  }
}

function expandPlaceholdersInObject(obj: object, slabs: TaggedSlab[]) {
  const rec = obj as Record<string, unknown>;
  for (const key in rec) {
    if (!Object.hasOwn(rec, key)) {
      continue;
    }
    const el = rec[key];
    if (typeof el === 'object' && el !== null) {
      const slabIndex = expandPlaceholdersOrReturnSlabIndex(el, slabs);
      if (slabIndex !== -1) {
        rec[key] = resolveSlab(slabs, slabIndex);
      }
    }
  }
}

// If `obj` is itself a placeholder, returns its slab index (the caller is
// responsible for replacing it in its parent). Otherwise, expands placeholders
// inside `obj` in place and returns -1.
function expandPlaceholdersOrReturnSlabIndex(
  obj: object,
  slabs: TaggedSlab[],
): number {
  if (Array.isArray(obj)) {
    expandPlaceholdersInArray(obj, slabs);
    return -1;
  }

  // Detect placeholders. These are objects whose only own key is "$s".
  if (!Object.hasOwn(obj, '$s') || Object.keys(obj).length !== 1) {
    expandPlaceholdersInObject(obj, slabs);
    return -1;
  }

  // We have a placeholder object!
  const $s = (obj as Record<string, unknown>)['$s'];
  if (!Number.isInteger($s)) {
    throw new Error(
      `Encountered slab placeholder with non-integer slab index ${String($s)}`,
    );
  }
  const idx = $s as number;
  if (idx < 0 || idx >= slabs.length) {
    throw new Error(`Unexpected slab index ${idx}`);
  }
  return idx;
}
