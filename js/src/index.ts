/**
 * json-slabs — generic "JSON with binary slabs" serialization.
 *
 * Any JavaScript object that contains TypedArrays can be serialized to a
 * compact binary blob and restored losslessly. TypedArrays anywhere in the
 * object tree are lifted out into raw binary slabs; their positions in the
 * JSON are replaced by { "$s": N } placeholders.
 *
 * High-level API:
 *   encode(obj, splitOut?)        — object → Uint8Array binary blob
 *   encodeToBlob(obj, splitOut?)  — object → Blob (avoids the large allocation / copy that `encode` performs; suitable for streams)
 *   decode<T>(buffer)             — Uint8Array binary blob → T
 *   new Builder()                 — low-level builder for manual slab construction
 *   decodeContainer(buffer)       — low-level reader returning the raw slab table
 *
 * `splitOut` is an optional list of nested values (matched by reference
 * identity) that each get lifted out of the root JSON into their own
 * SlabType.Json sub-slab. A `{ "$s": N }` placeholder is left in the parent
 * JSON in their place. See `encode` for the full rules.
 *
 * Container layout:
 *   [0..7]   Magic (8 bytes): 0xDC 0xDF "JSLB" 0x01 0x00
 *   [8..11]  uint32LE version = 1
 *   [12..15] uint32LE slab count
 *   [16..19] uint32LE root JSON slab index
 *   [20..]   Slab table: for each slab, type(u32LE) + startOffset(u32LE) + byteLength(u32LE)
 *   [..]     Slab data, each aligned to its element size; gaps zero-filled.
 */

// ── Constants ──────────────────────────────────────────────────────────────

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

// ── Helpers ────────────────────────────────────────────────────────────────

function alignUp(pos: number, alignment: number): number {
  return (pos + alignment - 1) & ~(alignment - 1);
}

// Element size in bytes for a slab type. Equal to the alignment for all
// numeric types; Uint8 and Json are byte-granular. Unknown types fall through
// to 1; the decoder's element-size check validates byteLength against this.
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

// ── Public types ───────────────────────────────────────────────────────────

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

export type SlabPlaceholder = { $s: number };

export type DecodedContainer = {
  /**
   * All slabs in order, as zero-copy views into the input buffer.
   * `SlabType.Uint8` and `SlabType.Json` slabs both decode to `Uint8Array` —
   * use the parallel `slabTypes[i]` to disambiguate, or call `jsonSlabBytes`
   * for the JSON case.
   */
  slabs: AnySlab[];
  /** Parallel to `slabs`: the `SlabType` for each slab. */
  slabTypes: SlabType[];
  rootJsonSlabIndex: number;
};

// ── Builder ────────────────────────────────────────────────────────────────

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
  addSlab(slab: AnySlab): SlabPlaceholder {
    this._checkNotFinished();
    if (slab instanceof Uint8ClampedArray) {
      throw new TypeError(
        'json-slabs does not support Uint8ClampedArray. Copy to a Uint8Array first.',
      );
    }
    if (slab instanceof Int8Array) return this._push(SlabType.Int8, slab);
    if (slab instanceof Uint8Array) return this._push(SlabType.Uint8, slab);
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

  /** Register a nested JSON document as a slab. UTF-8 encoded if a string. */
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

// ── decodeContainer ────────────────────────────────────────────────────────

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

  // Read slab table, validating each entry as we go.
  const slabTypes: number[] = new Array(slabCount);
  const slabByteLengths: number[] = new Array(slabCount);
  const slabStartOffsets: number[] = new Array(slabCount);
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
    slabTypes[i] = typeByte;
    slabByteLengths[i] = byteLen;
    slabStartOffsets[i] = startOff;
  }

  if (slabTypes[rootJsonSlabIndex] !== SlabType.Json) {
    throw new Error(
      `JSLB rootJsonSlabIndex=${rootJsonSlabIndex} points to slab of type ${slabTypes[rootJsonSlabIndex]}, expected SlabType.Json (${SlabType.Json})`,
    );
  }

  // Reconstruct typed array views into the buffer (zero-copy).
  // startOffset is relative to the container start; add buffer.byteOffset for
  // the absolute position within the underlying ArrayBuffer.
  const slabs: AnySlab[] = new Array(slabCount);
  for (let i = 0; i < slabCount; i++) {
    const absOffset = buffer.byteOffset + slabStartOffsets[i]!;
    slabs[i] = slabView(
      buffer.buffer,
      absOffset,
      slabTypes[i]!,
      slabByteLengths[i]!,
    );
  }

  return {
    slabs,
    slabTypes: slabTypes as SlabType[],
    rootJsonSlabIndex,
  };
}

function slabView(
  ab: ArrayBufferLike,
  offset: number,
  typeByte: number,
  byteLength: number,
): AnySlab {
  switch (typeByte) {
    case SlabType.Int8:
      return new Int8Array(ab, offset, byteLength);
    case SlabType.Int16:
      return new Int16Array(ab, offset, byteLength / 2);
    case SlabType.Uint16:
      return new Uint16Array(ab, offset, byteLength / 2);
    case SlabType.Int32:
      return new Int32Array(ab, offset, byteLength / 4);
    case SlabType.Uint32:
      return new Uint32Array(ab, offset, byteLength / 4);
    case SlabType.Float32:
      return new Float32Array(ab, offset, byteLength / 4);
    case SlabType.Float64:
      return new Float64Array(ab, offset, byteLength / 8);
    case SlabType.Int64:
      return new BigInt64Array(ab, offset, byteLength / 8);
    case SlabType.Uint64:
      return new BigUint64Array(ab, offset, byteLength / 8);
    default:
      return new Uint8Array(ab, offset, byteLength); // SlabType.Uint8, SlabType.Json
  }
}

/** Returns the raw bytes if the slab is a `SlabType.Json` slab, else `null`. */
export function jsonSlabBytes(
  c: DecodedContainer,
  index: number,
): Uint8Array | null {
  return c.slabTypes[index] === SlabType.Json
    ? (c.slabs[index] as Uint8Array)
    : null;
}

// ── High-level API ─────────────────────────────────────────────────────────

function stringifyWithSlabs(
  obj: unknown,
  builder: Builder,
  splitOut?: ReadonlyArray<unknown>,
): string {
  const splitSet =
    splitOut && splitOut.length > 0 ? new Set<unknown>(splitOut) : null;

  function go(value: unknown): string {
    // Track the top-level call so a value passed in `splitOut` that happens
    // to also be `value` itself is not split into a sub-slab — the root JSON
    // must not be a placeholder. We don't use `key === ''` because that
    // would also false-match user data shaped `{ '': nested }`.
    let isTop = true;
    return JSON.stringify(value, function (_key, val) {
      const atTop = isTop;
      isTop = false;
      if (val instanceof Uint8ClampedArray) {
        throw new TypeError(
          'json-slabs does not support Uint8ClampedArray. Copy to a Uint8Array first.',
        );
      }
      if (ArrayBuffer.isView(val) && !(val instanceof DataView)) {
        return builder.addSlab(val as AnySlab);
      }
      if (
        !atTop &&
        splitSet !== null &&
        val !== null &&
        typeof val === 'object' &&
        splitSet.has(val)
      ) {
        return builder.addJsonSlab(go(val));
      }
      return val;
    });
  }

  return go(obj);
}

/**
 * Serialize any object to a binary blob.
 * TypedArrays anywhere in the tree are extracted as binary slabs and
 * replaced by `{ "$s": N }` placeholders in the JSON skeleton.
 *
 * `splitOut`, if provided, is a list of nested values within `obj` to lift
 * out of the root JSON into their own `SlabType.Json` sub-slabs. Rules:
 *
 *   1. Matching is by reference identity (`===` / Set membership).
 *   2. Each value must be reachable from `obj`; unreachable entries
 *      silently have no effect.
 *   3. If `obj` itself appears in the list, it is ignored — the top-level
 *      value is always the root JSON, never split into a sub-slab.
 *   4. TypedArrays in this list are still encoded as their native typed
 *      slab. `splitOut` only affects non-TypedArray values.
 */
export function encode(
  obj: unknown,
  splitOut?: ReadonlyArray<unknown>,
): Uint8Array<ArrayBuffer> {
  const builder = new Builder();
  return builder.toBuffer(stringifyWithSlabs(obj, builder, splitOut));
}

/**
 * Serialize any object to a Blob, avoiding allocation of a single
 * concatenated buffer. Suitable for piping through a CompressionStream
 * or passing to fetch() / Response without extra copies.
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
 * Deserialize a binary blob back to an object.
 * `{ "$s": N }` placeholders are replaced with TypedArray views, or for
 * `SlabType.Json` slabs, with recursively parsed JSON objects (sharing the
 * same slab index space).
 *
 * The optional type parameter `T` lets callers express the expected shape
 * without a separate cast: `decode<MyType>(blob)`.
 */
export function decode<T = unknown>(buffer: Uint8Array): T {
  const { slabs, slabTypes, rootJsonSlabIndex } = decodeContainer(buffer);
  const decoder = new TextDecoder();
  const reviver = (_key: string, value: unknown): unknown => {
    if (
      value !== null &&
      typeof value === 'object' &&
      !Array.isArray(value) &&
      '$s' in (value as Record<string, unknown>)
    ) {
      const idx = (value as SlabPlaceholder).$s;
      if (slabTypes[idx] === SlabType.Json) {
        return JSON.parse(decoder.decode(slabs[idx] as Uint8Array), reviver);
      }
      return slabs[idx];
    }
    return value;
  };
  return JSON.parse(
    decoder.decode(slabs[rootJsonSlabIndex] as Uint8Array),
    reviver,
  ) as T;
}
