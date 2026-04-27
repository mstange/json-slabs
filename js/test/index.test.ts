import { describe, it, expect } from 'vitest';
import {
  slabify,
  slabifyToBlob,
  parse,
  Builder,
  decode,
  isJsonSlabsFile,
  TYPE_INT8,
  TYPE_UINT8,
  TYPE_INT16,
  TYPE_UINT16,
  TYPE_INT32,
  TYPE_UINT32,
  TYPE_FLOAT32,
  TYPE_FLOAT64,
  TYPE_INT64,
  TYPE_UINT64,
  TYPE_JSON,
} from '../src/index.js';

describe('round-trip', () => {
  it('round-trips a plain object with no TypedArrays', () => {
    const input = {
      name: 'example',
      count: 42,
      ratio: 3.14,
      flag: true,
      nothing: null,
      nested: { a: [1, 2, 3], b: 'hi' },
    };
    expect(parse(slabify(input))).toEqual(input);
  });

  it('round-trips primitive scalars at the root', () => {
    expect(parse(slabify(42))).toBe(42);
    expect(parse(slabify('hi'))).toBe('hi');
    expect(parse(slabify(true))).toBe(true);
    expect(parse(slabify(null))).toBe(null);
  });

  it('preserves key order in objects', () => {
    const input = { z: 1, a: 2, m: 3 };
    const out = parse<Record<string, number>>(slabify(input));
    expect(Object.keys(out)).toEqual(['z', 'a', 'm']);
  });

  it('round-trips every TypedArray type', () => {
    const input = {
      i8: new Int8Array([-128, 0, 127]),
      u8: new Uint8Array([0, 128, 255]),
      i16: new Int16Array([-32768, 0, 32767]),
      u16: new Uint16Array([0, 1, 65535]),
      i32: new Int32Array([-2147483648, 0, 2147483647]),
      u32: new Uint32Array([0, 1, 4294967295]),
      f32: new Float32Array([1.5, -2.25, 0]),
      f64: new Float64Array([1.5, -2.25, Math.PI, Number.MAX_VALUE]),
      i64: new BigInt64Array([-1n, 0n, 9223372036854775807n]),
      u64: new BigUint64Array([0n, 1n, 18446744073709551615n]),
    };

    const out = parse<typeof input>(slabify(input));

    expect(out.i8).toBeInstanceOf(Int8Array);
    expect(out.u8).toBeInstanceOf(Uint8Array);
    expect(out.i16).toBeInstanceOf(Int16Array);
    expect(out.u16).toBeInstanceOf(Uint16Array);
    expect(out.i32).toBeInstanceOf(Int32Array);
    expect(out.u32).toBeInstanceOf(Uint32Array);
    expect(out.f32).toBeInstanceOf(Float32Array);
    expect(out.f64).toBeInstanceOf(Float64Array);
    expect(out.i64).toBeInstanceOf(BigInt64Array);
    expect(out.u64).toBeInstanceOf(BigUint64Array);

    expect(Array.from(out.i8)).toEqual(Array.from(input.i8));
    expect(Array.from(out.u8)).toEqual(Array.from(input.u8));
    expect(Array.from(out.i16)).toEqual(Array.from(input.i16));
    expect(Array.from(out.u16)).toEqual(Array.from(input.u16));
    expect(Array.from(out.i32)).toEqual(Array.from(input.i32));
    expect(Array.from(out.u32)).toEqual(Array.from(input.u32));
    expect(Array.from(out.f32)).toEqual(Array.from(input.f32));
    expect(Array.from(out.f64)).toEqual(Array.from(input.f64));
    expect(Array.from(out.i64)).toEqual(Array.from(input.i64));
    expect(Array.from(out.u64)).toEqual(Array.from(input.u64));
  });

  it('round-trips empty TypedArrays', () => {
    const input = { empty: new Float64Array(0), nonEmpty: new Int32Array([1]) };
    const out = parse<typeof input>(slabify(input));
    expect(out.empty).toBeInstanceOf(Float64Array);
    expect(out.empty.length).toBe(0);
    expect(Array.from(out.nonEmpty)).toEqual([1]);
  });

  it('handles deeply nested TypedArrays', () => {
    const input = {
      level1: {
        level2: {
          level3: { values: new Float32Array([1, 2, 3]) },
        },
        siblings: [
          new Int32Array([10, 20]),
          new Int32Array([30, 40]),
          { weights: new Float64Array([0.1, 0.9]) },
        ],
      },
    };
    const out = parse<any>(slabify(input));
    expect(Array.from(out.level1.level2.level3.values)).toEqual([1, 2, 3]);
    expect(Array.from(out.level1.siblings[0])).toEqual([10, 20]);
    expect(Array.from(out.level1.siblings[1])).toEqual([30, 40]);
    expect(Array.from(out.level1.siblings[2].weights)).toEqual([0.1, 0.9]);
  });

  it('handles arrays of TypedArrays', () => {
    const input = [
      new Int32Array([1, 2]),
      new Int32Array([3, 4, 5]),
      new Int32Array([]),
    ];
    const out = parse<Int32Array[]>(slabify(input));
    expect(out.length).toBe(3);
    expect(Array.from(out[0]!)).toEqual([1, 2]);
    expect(Array.from(out[1]!)).toEqual([3, 4, 5]);
    expect(Array.from(out[2]!)).toEqual([]);
  });
});

describe('alignment', () => {
  it('aligns Float64 slabs to 8 bytes', () => {
    const input = {
      tag: new Uint8Array([1]),
      values: new Float64Array([1.5, 2.5]),
    };
    const decoded = decode(slabify(input));
    const f64Idx = decoded.slabTypes.indexOf(TYPE_FLOAT64);
    expect(f64Idx).toBeGreaterThanOrEqual(0);
    expect(decoded.slabs[f64Idx]!.byteOffset % 8).toBe(0);
  });

  it('aligns Int32 slabs to 4 bytes after a 1-byte slab', () => {
    const input = {
      tag: new Uint8Array([1]),
      values: new Int32Array([100, 200, 300]),
    };
    const decoded = decode(slabify(input));
    const i32Idx = decoded.slabTypes.indexOf(TYPE_INT32);
    expect(decoded.slabs[i32Idx]!.byteOffset % 4).toBe(0);
  });
});

describe('container header', () => {
  it('starts with the JSLB magic', () => {
    const blob = slabify({ a: 1 });
    expect(Array.from(blob.slice(0, 8))).toEqual([
      0xdc, 0xdf, 0x4a, 0x53, 0x4c, 0x42, 0x01, 0x00,
    ]);
  });

  it('throws on bad magic bytes', () => {
    const bad = new Uint8Array(40);
    expect(() => parse(bad)).toThrow(/bad magic/i);
  });

  it('throws on unsupported version', () => {
    const blob = slabify({ a: 1 });
    blob[8] = 99;
    expect(() => parse(blob)).toThrow(/version/i);
  });
});

describe('isJsonSlabsFile', () => {
  it('returns true for a real JSLB blob', () => {
    expect(isJsonSlabsFile(slabify({ a: 1 }))).toBe(true);
  });

  it('returns true regardless of version', () => {
    const blob = slabify({ a: 1 });
    blob[8] = 99;
    expect(isJsonSlabsFile(blob)).toBe(true);
  });

  it('returns false for buffers shorter than the magic', () => {
    expect(isJsonSlabsFile(new Uint8Array(0))).toBe(false);
    expect(isJsonSlabsFile(new Uint8Array(7))).toBe(false);
  });

  it('returns false when magic bytes do not match', () => {
    expect(isJsonSlabsFile(new Uint8Array(40))).toBe(false);
  });
});

describe('zero-copy views', () => {
  it('TypedArray views share the backing buffer with the input blob', () => {
    const input = { values: new Int32Array([1, 2, 3, 4]) };
    const blob = slabify(input);
    const out = parse<typeof input>(blob);
    expect(out.values.buffer).toBe(blob.buffer);
  });

  it('mutation through the view is visible in the underlying buffer', () => {
    const blob = slabify({ values: new Int32Array([1, 2, 3]) });
    const out = parse<{ values: Int32Array }>(blob);
    out.values[0] = 999;
    const out2 = parse<{ values: Int32Array }>(blob);
    expect(out2.values[0]).toBe(999);
  });
});

describe('slabifyToBlob', () => {
  it('produces a Blob that decodes identically to slabify', async () => {
    const input = {
      label: 'test',
      ints: new Int32Array([1, 2, 3]),
      floats: new Float64Array([0.1, 0.2]),
    };
    const direct = slabify(input);
    const asBlob = slabifyToBlob(input);
    const fromBlob = new Uint8Array(await asBlob.arrayBuffer());
    expect(fromBlob.length).toBe(direct.length);
    expect(Array.from(fromBlob)).toEqual(Array.from(direct));
  });
});

describe('subSlabs', () => {
  it('lifts a nested object into its own TYPE_JSON sub-slab', () => {
    const shared = { stringArray: ['hello', 'world'] };
    const data = { libs: [], shared };
    const blob = slabify(data, [shared.stringArray]);

    const decoded = decode(blob);
    const jsonSlabCount = decoded.slabTypes.filter(
      (t) => t === TYPE_JSON,
    ).length;
    expect(jsonSlabCount).toBe(2); // root + sub-slab

    expect(parse(blob)).toEqual(data);
  });

  it('subSlabs roundtrip preserves TypedArrays inside the sub-slab', () => {
    const sub = { weights: new Float64Array([0.5, 1.5]) };
    const data = { meta: 'x', sub };
    const out = parse<typeof data>(slabify(data, [sub]));
    expect(Array.from(out.sub.weights)).toEqual([0.5, 1.5]);
  });

  it('does not split the top-level value even if listed in subSlabs', () => {
    const data = { a: 1, b: 2 };
    const blob = slabify(data, [data]);
    const decoded = decode(blob);
    const jsonSlabCount = decoded.slabTypes.filter(
      (t) => t === TYPE_JSON,
    ).length;
    expect(jsonSlabCount).toBe(1);
    expect(parse(blob)).toEqual(data);
  });
});

describe('Builder', () => {
  function concat(chunks: Uint8Array[]): Uint8Array {
    const total = chunks.reduce((s, c) => s + c.byteLength, 0);
    const out = new Uint8Array(total);
    let off = 0;
    for (const c of chunks) {
      out.set(c, off);
      off += c.byteLength;
    }
    return out;
  }

  it('builds a container manually with placeholders', () => {
    const builder = new Builder();
    const p1 = builder.addI32Slab(new Int32Array([1, 2, 3]));
    const p2 = builder.addF64Slab(new Float64Array([1.5, 2.5]));
    expect(p1.$s).toBe(0);
    expect(p2.$s).toBe(1);

    const skeleton = { vals: p1, weights: p2 };
    const jsonBytes = new TextEncoder().encode(JSON.stringify(skeleton));
    const merged = concat(builder.finish(jsonBytes));

    const out = parse<{ vals: Int32Array; weights: Float64Array }>(merged);
    expect(Array.from(out.vals)).toEqual([1, 2, 3]);
    expect(Array.from(out.weights)).toEqual([1.5, 2.5]);
  });

  it('exposes addI64Slab / addU64Slab (renamed from BigI64/BigU64)', () => {
    const builder = new Builder();
    const p1 = builder.addI64Slab(new BigInt64Array([1n, 2n]));
    const p2 = builder.addU64Slab(new BigUint64Array([3n, 4n]));
    const skeleton = { a: p1, b: p2 };
    const jsonBytes = new TextEncoder().encode(JSON.stringify(skeleton));
    const merged = concat(builder.finish(jsonBytes));
    const out = parse<{ a: BigInt64Array; b: BigUint64Array }>(merged);
    expect(Array.from(out.a)).toEqual([1n, 2n]);
    expect(Array.from(out.b)).toEqual([3n, 4n]);
  });

  it('addJsonSlab registers a sub-slab that recursively parses on decode', () => {
    const builder = new Builder();
    const subJson = new TextEncoder().encode(JSON.stringify(['a', 'b', 'c']));
    const subPlaceholder = builder.addJsonSlab(subJson);
    const rootJson = new TextEncoder().encode(
      JSON.stringify({ items: subPlaceholder }),
    );
    const merged = concat(builder.finish(rootJson));

    const out = parse<{ items: string[] }>(merged);
    expect(out.items).toEqual(['a', 'b', 'c']);
  });
});

describe('decode (low-level)', () => {
  it('exposes type constants and slab types', () => {
    const blob = slabify({
      a: new Int8Array([1]),
      b: new Uint16Array([2]),
      c: new Float32Array([3]),
    });
    const decoded = decode(blob);
    expect(decoded.slabTypes).toContain(TYPE_INT8);
    expect(decoded.slabTypes).toContain(TYPE_UINT16);
    expect(decoded.slabTypes).toContain(TYPE_FLOAT32);
    expect(decoded.slabTypes[decoded.rootJsonSlabIndex]).toBe(TYPE_JSON);
  });

  it('TYPE_* constants have the expected values', () => {
    expect(TYPE_INT8).toBe(0x00);
    expect(TYPE_UINT8).toBe(0x01);
    expect(TYPE_INT16).toBe(0x02);
    expect(TYPE_UINT16).toBe(0x03);
    expect(TYPE_INT32).toBe(0x04);
    expect(TYPE_UINT32).toBe(0x05);
    expect(TYPE_FLOAT32).toBe(0x06);
    expect(TYPE_FLOAT64).toBe(0x07);
    expect(TYPE_INT64).toBe(0x08);
    expect(TYPE_UINT64).toBe(0x09);
    expect(TYPE_JSON).toBe(0x0a);
  });
});

describe('parsing from a sub-buffer', () => {
  it('decodes correctly when the input is a Uint8Array view at an 8-byte-aligned non-zero offset', () => {
    const input = { values: new Int32Array([7, 8, 9]) };
    const inner = slabify(input);

    const outer = new Uint8Array(inner.byteLength + 32);
    outer.set(inner, 16);
    const view = outer.subarray(16, 16 + inner.byteLength);

    const out = parse<typeof input>(view);
    expect(Array.from(out.values)).toEqual([7, 8, 9]);
  });

  it('throws an actionable error when the view is at a non-aligned offset', () => {
    const input = { values: new Int32Array([7, 8, 9]) };
    const inner = slabify(input);
    const outer = new Uint8Array(inner.byteLength + 32);
    outer.set(inner, 17);
    const misaligned = outer.subarray(17, 17 + inner.byteLength);

    expect(() => parse(misaligned)).toThrow(/8-byte-aligned/);
    expect(() => parse(misaligned)).toThrow(/byteOffset=17/);
  });
});

describe('malformed-input safety', () => {
  function tamperHeader(
    blob: Uint8Array<ArrayBuffer>,
    field: 'slabCount' | 'rootIdx',
    value: number,
  ): Uint8Array {
    const copy = new Uint8Array(blob);
    const dv = new DataView(copy.buffer);
    if (field === 'slabCount') dv.setUint32(12, value, true);
    if (field === 'rootIdx') dv.setUint32(16, value, true);
    return copy;
  }

  it('throws on a buffer shorter than the fixed header', () => {
    expect(() => parse(new Uint8Array(10))).toThrow(/too short/i);
  });

  it('throws when slabCount is so large the slab table overruns the buffer', () => {
    const blob = slabify({ a: new Int32Array([1, 2]) });
    const bad = tamperHeader(blob, 'slabCount', 0x7fffffff);
    expect(() => parse(bad)).toThrow(/slab table overruns/i);
  });

  it('throws when rootJsonSlabIndex is out of range', () => {
    const blob = slabify({ a: 1 });
    const bad = tamperHeader(blob, 'rootIdx', 999);
    expect(() => parse(bad)).toThrow(/rootJsonSlabIndex=999 out of range/);
  });

  it('throws when a slab table entry overruns the buffer', () => {
    const blob = slabify({ a: new Int32Array([1, 2, 3]) });
    const bad = new Uint8Array(blob);
    // First slab table entry: byteLength field is at table offset +8 (= 28).
    new DataView(bad.buffer).setUint32(28, 0x7fffffff, true);
    expect(() => parse(bad)).toThrow(/slab 0 overruns buffer/);
  });

  it('throws when a slab byteLength is not a multiple of its element size', () => {
    const blob = slabify({ a: new Int32Array([1, 2, 3]) });
    const bad = new Uint8Array(blob);
    // First slab is the Int32Array with byteLength=12. Tamper to 11.
    new DataView(bad.buffer).setUint32(28, 11, true);
    expect(() => parse(bad)).toThrow(/not a multiple of element size 4/);
  });

  it('throws when rootJsonSlabIndex points to a non-JSON slab', () => {
    const blob = slabify({ a: new Int32Array([1, 2]) });
    // Two slabs: [0] = Int32Array, [1] = JSON (root). Point root at slab 0.
    const bad = tamperHeader(blob, 'rootIdx', 0);
    expect(() => parse(bad)).toThrow(/expected TYPE_JSON/);
  });
});
