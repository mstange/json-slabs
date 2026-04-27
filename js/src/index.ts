/**
 * json-slabs — generic "JSON with binary slabs" serialization.
 *
 * Any JavaScript object that contains TypedArrays can be serialized to a
 * compact binary blob and restored losslessly. TypedArrays anywhere in the
 * object tree are lifted out into raw binary slabs; their positions in the
 * JSON are replaced by { "$s": N } placeholders.
 *
 * High-level API:
 *   slabify(obj, subSlabs?)       — object → Uint8Array binary blob
 *   slabifyToBlob(obj, subSlabs?) — object → Blob (built from list of buffers, avoids large allocation / copy when used with streams)
 *   parse<T>(buffer)              — Uint8Array binary blob → T
 *   new Builder()                 — low-level builder for manual slab construction
 *
 * `subSlabs` is an optional list of nested values (matched by reference
 * identity) that should each be lifted out into their own TYPE_JSON sub-slab,
 * leaving a { "$s": N } placeholder in the parent JSON.
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

export const TYPE_INT8 = 0x00; // Int8Array
export const TYPE_UINT8 = 0x01; // Uint8Array
export const TYPE_INT16 = 0x02; // Int16Array
export const TYPE_UINT16 = 0x03; // Uint16Array
export const TYPE_INT32 = 0x04; // Int32Array
export const TYPE_UINT32 = 0x05; // Uint32Array
export const TYPE_FLOAT32 = 0x06; // Float32Array
export const TYPE_FLOAT64 = 0x07; // Float64Array
export const TYPE_INT64 = 0x08; // BigInt64Array
export const TYPE_UINT64 = 0x09; // BigUint64Array
export const TYPE_JSON = 0x0a; // UTF-8 JSON bytes

// ── Helpers ────────────────────────────────────────────────────────────────

function alignUp(pos: number, alignment: number): number {
  return (pos + alignment - 1) & ~(alignment - 1);
}

// Element size in bytes for a slab type. Equal to the alignment for all
// numeric types; TYPE_UINT8 and TYPE_JSON are byte-granular.
function elementSizeForTypeByte(typeByte: number): number {
  switch (typeByte) {
    case TYPE_INT16:
    case TYPE_UINT16:
      return 2;
    case TYPE_INT32:
    case TYPE_UINT32:
    case TYPE_FLOAT32:
      return 4;
    case TYPE_FLOAT64:
    case TYPE_INT64:
    case TYPE_UINT64:
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
  jsonBytes: Uint8Array;
  /**
   * All slabs in order, as zero-copy views into the input buffer.
   * TYPE_UINT8 and TYPE_JSON slabs both decode to `Uint8Array` — use the
   * parallel `slabTypes[i]` to disambiguate.
   */
  slabs: AnySlab[];
  /** Parallel to `slabs`: the TYPE_* constant for each slab. */
  slabTypes: number[];
  rootJsonSlabIndex: number;
};

// ── Builder ────────────────────────────────────────────────────────────────

export class Builder {
  private readonly _entries: Array<{
    typeByte: number;
    view: ArrayBufferView;
  }> = [];

  addI8Slab(slab: Int8Array): SlabPlaceholder {
    return this._push(TYPE_INT8, slab);
  }
  addU8Slab(slab: Uint8Array): SlabPlaceholder {
    return this._push(TYPE_UINT8, slab);
  }
  addI16Slab(slab: Int16Array): SlabPlaceholder {
    return this._push(TYPE_INT16, slab);
  }
  addU16Slab(slab: Uint16Array): SlabPlaceholder {
    return this._push(TYPE_UINT16, slab);
  }
  addI32Slab(slab: Int32Array): SlabPlaceholder {
    return this._push(TYPE_INT32, slab);
  }
  addU32Slab(slab: Uint32Array): SlabPlaceholder {
    return this._push(TYPE_UINT32, slab);
  }
  addF32Slab(slab: Float32Array): SlabPlaceholder {
    return this._push(TYPE_FLOAT32, slab);
  }
  addF64Slab(slab: Float64Array): SlabPlaceholder {
    return this._push(TYPE_FLOAT64, slab);
  }
  addI64Slab(slab: BigInt64Array): SlabPlaceholder {
    return this._push(TYPE_INT64, slab);
  }
  addU64Slab(slab: BigUint64Array): SlabPlaceholder {
    return this._push(TYPE_UINT64, slab);
  }
  /** Register a nested JSON document (UTF-8 bytes) as a slab. */
  addJsonSlab(jsonBytes: Uint8Array): SlabPlaceholder {
    return this._push(TYPE_JSON, jsonBytes);
  }

  private _push(typeByte: number, view: ArrayBufferView): SlabPlaceholder {
    const bin = this._entries.length;
    this._entries.push({ typeByte, view });
    return { $s: bin };
  }

  /**
   * Appends the JSON slab and returns the container as a list of chunks.
   * Slab data is returned as zero-copy views; only the header is newly
   * allocated. Callers can stream the chunks or concatenate as needed.
   * The Builder must not be used after this call.
   */
  finish(jsonBytes: Uint8Array): Uint8Array[] {
    const rootJsonSlabIndex = this._entries.length;
    this._entries.push({ typeByte: TYPE_JSON, view: jsonBytes });

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
}

// ── decode ─────────────────────────────────────────────────────────────────

export function isJsonSlabsFile(buffer: Uint8Array): boolean {
  if (buffer.byteLength < MAGIC.length) return false;
  for (let i = 0; i < MAGIC.length; i++) {
    if (buffer[i] !== MAGIC[i]) return false;
  }
  return true;
}

export function decode(buffer: Uint8Array): DecodedContainer {
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

  if (slabTypes[rootJsonSlabIndex] !== TYPE_JSON) {
    throw new Error(
      `JSLB rootJsonSlabIndex=${rootJsonSlabIndex} points to slab of type ${slabTypes[rootJsonSlabIndex]}, expected TYPE_JSON (${TYPE_JSON})`,
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
    jsonBytes: slabs[rootJsonSlabIndex] as Uint8Array,
    slabs,
    slabTypes,
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
    case TYPE_INT8:
      return new Int8Array(ab, offset, byteLength);
    case TYPE_INT16:
      return new Int16Array(ab, offset, byteLength / 2);
    case TYPE_UINT16:
      return new Uint16Array(ab, offset, byteLength / 2);
    case TYPE_INT32:
      return new Int32Array(ab, offset, byteLength / 4);
    case TYPE_UINT32:
      return new Uint32Array(ab, offset, byteLength / 4);
    case TYPE_FLOAT32:
      return new Float32Array(ab, offset, byteLength / 4);
    case TYPE_FLOAT64:
      return new Float64Array(ab, offset, byteLength / 8);
    case TYPE_INT64:
      return new BigInt64Array(ab, offset, byteLength / 8);
    case TYPE_UINT64:
      return new BigUint64Array(ab, offset, byteLength / 8);
    default:
      return new Uint8Array(ab, offset, byteLength); // TYPE_UINT8, TYPE_JSON
  }
}

// ── High-level API ─────────────────────────────────────────────────────────

function _buildChunks(
  obj: unknown,
  subSlabs?: ReadonlyArray<unknown>,
): Uint8Array[] {
  const builder = new Builder();
  const splitSet =
    subSlabs && subSlabs.length > 0 ? new Set<unknown>(subSlabs) : null;
  const encoder = new TextEncoder();

  function encode(value: unknown): Uint8Array {
    // Track the top-level call so a value passed in `subSlabs` that happens to
    // also be `value` itself is not split into a sub-slab — the root JSON must
    // not be a placeholder. We don't use `key === ''` because that would also
    // false-match user data shaped `{ '': nested }`.
    let isTop = true;
    const jsonStr = JSON.stringify(value, function (_key, val) {
      const atTop = isTop;
      isTop = false;
      if (val instanceof Int8Array) return builder.addI8Slab(val);
      if (val instanceof Uint8Array) return builder.addU8Slab(val);
      if (val instanceof Int16Array) return builder.addI16Slab(val);
      if (val instanceof Uint16Array) return builder.addU16Slab(val);
      if (val instanceof Int32Array) return builder.addI32Slab(val);
      if (val instanceof Uint32Array) return builder.addU32Slab(val);
      if (val instanceof Float32Array) return builder.addF32Slab(val);
      if (val instanceof Float64Array) return builder.addF64Slab(val);
      if (val instanceof BigInt64Array) return builder.addI64Slab(val);
      if (val instanceof BigUint64Array) return builder.addU64Slab(val);
      if (
        !atTop &&
        splitSet !== null &&
        val !== null &&
        typeof val === 'object' &&
        splitSet.has(val)
      ) {
        return builder.addJsonSlab(encode(val));
      }
      return val;
    });
    return encoder.encode(jsonStr);
  }

  return builder.finish(encode(obj));
}

/**
 * Serialize any object to a binary blob.
 * TypedArrays anywhere in the tree are extracted as binary slabs and
 * replaced by `{ "$s": N }` placeholders in the JSON skeleton.
 *
 * `subSlabs`, if provided, is a list of nested object/array values within
 * `obj` that should each be lifted out into their own TYPE_JSON sub-slab
 * (matched by reference identity). A `{ "$s": N }` placeholder is left in
 * the parent JSON in their place. If `obj` itself appears in `subSlabs`,
 * it is ignored — the top-level value is always the root JSON, never split
 * into a sub-slab.
 */
export function slabify(
  obj: unknown,
  subSlabs?: ReadonlyArray<unknown>,
): Uint8Array<ArrayBuffer> {
  const chunks = _buildChunks(obj, subSlabs);
  const totalSize = chunks.reduce((sum, c) => sum + c.byteLength, 0);
  const out = new Uint8Array(totalSize);
  let off = 0;
  for (const c of chunks) {
    out.set(c, off);
    off += c.byteLength;
  }
  return out as Uint8Array<ArrayBuffer>;
}

/**
 * Serialize any object to a Blob, avoiding allocation of a single
 * concatenated buffer. Suitable for piping through a CompressionStream
 * or passing to fetch() / Response without extra copies.
 *
 * See `slabify` for the meaning of `subSlabs`.
 */
export function slabifyToBlob(
  obj: unknown,
  subSlabs?: ReadonlyArray<unknown>,
): Blob {
  return new Blob(_buildChunks(obj, subSlabs) as BlobPart[]);
}

/**
 * Deserialize a binary blob back to an object.
 * `{ "$s": N }` placeholders are replaced with TypedArray views, or for
 * TYPE_JSON slabs, with recursively parsed JSON objects (sharing the same
 * slab index space).
 *
 * The optional type parameter `T` lets callers express the expected shape
 * without a separate cast: `parse<MyType>(blob)`.
 */
export function parse<T = unknown>(buffer: Uint8Array): T {
  const { jsonBytes, slabs, slabTypes } = decode(buffer);
  const decoder = new TextDecoder();
  const reviver = (_key: string, value: unknown): unknown => {
    if (
      value !== null &&
      typeof value === 'object' &&
      !Array.isArray(value) &&
      '$s' in (value as Record<string, unknown>)
    ) {
      const idx = (value as SlabPlaceholder).$s;
      if (slabTypes[idx] === TYPE_JSON) {
        return JSON.parse(decoder.decode(slabs[idx] as Uint8Array), reviver);
      }
      return slabs[idx];
    }
    return value;
  };
  return JSON.parse(decoder.decode(jsonBytes), reviver) as T;
}
