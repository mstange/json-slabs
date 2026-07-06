import { describe, it, expect } from 'vitest';
import {
  encode,
  encodeToBlob,
  decode,
  Builder,
  decodeContainer,
  isJsonSlabsFile,
  SlabType,
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
    expect(decode(encode(input))).toEqual(input);
  });

  it('round-trips primitive scalars at the root', () => {
    expect(decode(encode(42))).toBe(42);
    expect(decode(encode('hi'))).toBe('hi');
    expect(decode(encode(true))).toBe(true);
    expect(decode(encode(null))).toBe(null);
  });

  it('preserves key order in objects', () => {
    const input = { z: 1, a: 2, m: 3 };
    const out = decode<Record<string, number>>(encode(input));
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

    const out = decode<typeof input>(encode(input));

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
    const out = decode<typeof input>(encode(input));
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
    const out = decode<any>(encode(input));
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
    const out = decode<Int32Array[]>(encode(input));
    expect(out.length).toBe(3);
    expect(Array.from(out[0]!)).toEqual([1, 2]);
    expect(Array.from(out[1]!)).toEqual([3, 4, 5]);
    expect(Array.from(out[2]!)).toEqual([]);
  });

  it('round-trips a top-level TypedArray', () => {
    const input = new Float64Array([1.5, 2.5, 3.5]);
    const out = decode<Float64Array>(encode(input));
    expect(out).toBeInstanceOf(Float64Array);
    expect(Array.from(out)).toEqual([1.5, 2.5, 3.5]);
  });
});

describe('alignment', () => {
  it('aligns Float64 slabs to 8 bytes', () => {
    const input = {
      tag: new Uint8Array([1]),
      values: new Float64Array([1.5, 2.5]),
    };
    const decoded = decodeContainer(encode(input));
    const f64Slab = decoded.slabs.find((s) => s.type === SlabType.Float64);
    if (f64Slab?.type !== SlabType.Float64) throw new Error('no Float64 slab');
    expect(f64Slab.array.byteOffset % 8).toBe(0);
  });

  it('aligns Int32 slabs to 4 bytes after a 1-byte slab', () => {
    const input = {
      tag: new Uint8Array([1]),
      values: new Int32Array([100, 200, 300]),
    };
    const decoded = decodeContainer(encode(input));
    const i32Slab = decoded.slabs.find((s) => s.type === SlabType.Int32);
    if (i32Slab?.type !== SlabType.Int32) throw new Error('no Int32 slab');
    expect(i32Slab.array.byteOffset % 4).toBe(0);
  });
});

describe('container header', () => {
  it('starts with the JSLB magic', () => {
    const blob = encode({ a: 1 });
    expect(Array.from(blob.slice(0, 8))).toEqual([
      0xdc, 0xdf, 0x4a, 0x53, 0x4c, 0x42, 0x01, 0x00,
    ]);
  });

  it('throws on bad magic bytes', () => {
    const bad = new Uint8Array(40);
    expect(() => decode(bad)).toThrow(/bad magic/i);
  });

  it('throws on unsupported version', () => {
    const blob = encode({ a: 1 });
    blob[8] = 99;
    expect(() => decode(blob)).toThrow(/version/i);
  });
});

describe('isJsonSlabsFile', () => {
  it('returns true for a real JSLB blob', () => {
    expect(isJsonSlabsFile(encode({ a: 1 }))).toBe(true);
  });

  it('returns true regardless of version', () => {
    const blob = encode({ a: 1 });
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
    const blob = encode(input);
    const out = decode<typeof input>(blob);
    expect(out.values.buffer).toBe(blob.buffer);
  });

  it('mutation through the view is visible in the underlying buffer', () => {
    const blob = encode({ values: new Int32Array([1, 2, 3]) });
    const out = decode<{ values: Int32Array }>(blob);
    out.values[0] = 999;
    const out2 = decode<{ values: Int32Array }>(blob);
    expect(out2.values[0]).toBe(999);
  });
});

describe('encodeToBlob', () => {
  it('produces a Blob that decodes identically to encode', async () => {
    const input = {
      label: 'test',
      ints: new Int32Array([1, 2, 3]),
      floats: new Float64Array([0.1, 0.2]),
    };
    const direct = encode(input);
    const asBlob = encodeToBlob(input);
    const fromBlob = new Uint8Array(await asBlob.arrayBuffer());
    expect(fromBlob.length).toBe(direct.length);
    expect(Array.from(fromBlob)).toEqual(Array.from(direct));
  });
});

describe('splitOut', () => {
  it('lifts a nested object into its own SlabType.Json sub-slab', () => {
    const shared = { stringArray: ['hello', 'world'] };
    const data = { libs: [], shared };
    const blob = encode(data, [shared.stringArray]);

    const decoded = decodeContainer(blob);
    const jsonSlabCount = decoded.slabs.filter(
      (s) => s.type === SlabType.Json,
    ).length;
    expect(jsonSlabCount).toBe(2); // root + sub-slab

    expect(decode(blob)).toEqual(data);
  });

  it('roundtrip preserves TypedArrays inside the sub-slab', () => {
    const sub = { weights: new Float64Array([0.5, 1.5]) };
    const data = { meta: 'x', sub };
    const out = decode<typeof data>(encode(data, [sub]));
    expect(Array.from(out.sub.weights)).toEqual([0.5, 1.5]);
  });

  it('does not split the top-level value even if listed in splitOut', () => {
    const data = { a: 1, b: 2 };
    const blob = encode(data, [data]);
    const decoded = decodeContainer(blob);
    const jsonSlabCount = decoded.slabs.filter(
      (s) => s.type === SlabType.Json,
    ).length;
    expect(jsonSlabCount).toBe(1);
    expect(decode(blob)).toEqual(data);
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
    const p1 = builder.addSlab(new Int32Array([1, 2, 3]));
    const p2 = builder.addSlab(new Float64Array([1.5, 2.5]));
    // Table index 0 is reserved for the root JSON; user-added slabs start at 1.
    expect(p1.$s).toBe(1);
    expect(p2.$s).toBe(2);

    const skeleton = { vals: p1, weights: p2 };
    const merged = builder.toBuffer(JSON.stringify(skeleton));

    const out = decode<{ vals: Int32Array; weights: Float64Array }>(merged);
    expect(Array.from(out.vals)).toEqual([1, 2, 3]);
    expect(Array.from(out.weights)).toEqual([1.5, 2.5]);
  });

  it('addSlab() dispatches BigInt64Array / BigUint64Array correctly', () => {
    const builder = new Builder();
    const p1 = builder.addSlab(new BigInt64Array([1n, 2n]));
    const p2 = builder.addSlab(new BigUint64Array([3n, 4n]));
    const skeleton = { a: p1, b: p2 };
    const merged = builder.toBuffer(JSON.stringify(skeleton));
    const out = decode<{ a: BigInt64Array; b: BigUint64Array }>(merged);
    expect(Array.from(out.a)).toEqual([1n, 2n]);
    expect(Array.from(out.b)).toEqual([3n, 4n]);
  });

  it('addJsonSlab registers a sub-slab that recursively parses on decode', () => {
    const builder = new Builder();
    const subPlaceholder = builder.addJsonSlab(JSON.stringify(['a', 'b', 'c']));
    const merged = builder.toBuffer(JSON.stringify({ items: subPlaceholder }));

    const out = decode<{ items: string[] }>(merged);
    expect(out.items).toEqual(['a', 'b', 'c']);
  });

  it('addJsonSlab accepts both string and Uint8Array', () => {
    const builder = new Builder();
    const a = builder.addJsonSlab('"from-string"');
    const b = builder.addJsonSlab(new TextEncoder().encode('"from-bytes"'));
    const merged = builder.toBuffer(JSON.stringify({ a, b }));
    expect(decode<{ a: string; b: string }>(merged)).toEqual({
      a: 'from-string',
      b: 'from-bytes',
    });
  });

  it('finish accepts a string for the root JSON', () => {
    const builder = new Builder();
    const p = builder.addSlab(new Int32Array([7, 8]));
    const merged = concat(builder.finish(JSON.stringify({ v: p })));
    expect(Array.from(decode<{ v: Int32Array }>(merged).v)).toEqual([7, 8]);
  });

  it('toBlob produces a Blob that decodes correctly', async () => {
    const builder = new Builder();
    const p = builder.addSlab(new Float32Array([1.5, 2.5]));
    const blob = builder.toBlob(JSON.stringify({ vals: p }));
    const bytes = new Uint8Array(await blob.arrayBuffer());
    const out = decode<{ vals: Float32Array }>(bytes);
    expect(Array.from(out.vals)).toEqual([1.5, 2.5]);
  });

  it('throws on use after finish', () => {
    const b1 = new Builder();
    b1.finish('null');
    expect(() => b1.addSlab(new Int32Array([1]))).toThrow(/already finished/);
    expect(() => b1.addJsonSlab('null')).toThrow(/already finished/);
    expect(() => b1.finish('null')).toThrow(/already finished/);
    expect(() => b1.toBuffer('null')).toThrow(/already finished/);
    expect(() => b1.toBlob('null')).toThrow(/already finished/);

    const b2 = new Builder();
    b2.toBuffer('null');
    expect(() => b2.addSlab(new Int32Array([1]))).toThrow(/already finished/);
  });

  it('accepts Uint8ClampedArray and stores it as a Uint8 slab', () => {
    const builder = new Builder();
    const placeholder = builder.addSlab(new Uint8ClampedArray([1, 2, 3]));
    const bytes = builder.toBuffer(JSON.stringify({ v: placeholder }));
    const decoded = decodeContainer(bytes);
    const u8Slab = decoded.slabs.find((s) => s.type === SlabType.Uint8);
    if (u8Slab?.type !== SlabType.Uint8) throw new Error('no Uint8 slab');
    expect(Array.from(u8Slab.array)).toEqual([1, 2, 3]);
  });
});

describe('decodeContainer', () => {
  it('exposes tagged slabs with type discriminators', () => {
    const blob = encode({
      a: new Int8Array([1]),
      b: new Uint16Array([2]),
      c: new Float32Array([3]),
    });
    const decoded = decodeContainer(blob);
    const types = decoded.slabs.map((s) => s.type);
    expect(types).toContain(SlabType.Int8);
    expect(types).toContain(SlabType.Uint16);
    expect(types).toContain(SlabType.Float32);
    expect(decoded.slabs[decoded.rootJsonSlabIndex]!.type).toBe(SlabType.Json);
  });

  it('exposes rootJsonBytes pointing at the root JSON slab', () => {
    const blob = encode({ v: new Int32Array([1, 2]) });
    const decoded = decodeContainer(blob);
    const root = decoded.slabs[decoded.rootJsonSlabIndex]!;
    expect(root.type).toBe(SlabType.Json);
    if (root.type === SlabType.Json) {
      expect(decoded.rootJsonBytes).toBe(root.jsonBytes);
    }
    const rootText = new TextDecoder().decode(decoded.rootJsonBytes);
    expect(JSON.parse(rootText)).toEqual({ v: { $s: 1 } });
  });

  it('SlabType.Json is the wire-protocol-stable value 0x0a', () => {
    expect(SlabType.Json).toBe(0x0a);
  });
});

describe('top-level encode accepts Uint8ClampedArray', () => {
  it('round-trips the bytes as a Uint8Array', () => {
    const bytes = encode({ x: new Uint8ClampedArray([1, 2, 3]) });
    const out = decode<{ x: Uint8Array }>(bytes);
    expect(out.x).toBeInstanceOf(Uint8Array);
    expect(Array.from(out.x)).toEqual([1, 2, 3]);
  });
});

describe('parsing from a sub-buffer', () => {
  it('decodes correctly when the input is a Uint8Array view at an 8-byte-aligned non-zero offset', () => {
    const input = { values: new Int32Array([7, 8, 9]) };
    const inner = encode(input);

    const outer = new Uint8Array(inner.byteLength + 32);
    outer.set(inner, 16);
    const view = outer.subarray(16, 16 + inner.byteLength);

    const out = decode<typeof input>(view);
    expect(Array.from(out.values)).toEqual([7, 8, 9]);
  });

  it('throws an actionable error when the view is at a non-aligned offset', () => {
    const input = { values: new Int32Array([7, 8, 9]) };
    const inner = encode(input);
    const outer = new Uint8Array(inner.byteLength + 32);
    outer.set(inner, 17);
    const misaligned = outer.subarray(17, 17 + inner.byteLength);

    expect(() => decode(misaligned)).toThrow(/8-byte-aligned/);
    expect(() => decode(misaligned)).toThrow(/byteOffset=17/);
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
    expect(() => decode(new Uint8Array(10))).toThrow(/too short/i);
  });

  it('throws when slabCount is so large the slab table overruns the buffer', () => {
    const blob = encode({ a: new Int32Array([1, 2]) });
    const bad = tamperHeader(blob, 'slabCount', 0x7fffffff);
    expect(() => decode(bad)).toThrow(/slab table overruns/i);
  });

  it('throws when rootJsonSlabIndex is out of range', () => {
    const blob = encode({ a: 1 });
    const bad = tamperHeader(blob, 'rootIdx', 999);
    expect(() => decode(bad)).toThrow(/rootJsonSlabIndex=999 out of range/);
  });

  it('throws when a slab table entry overruns the buffer', () => {
    const blob = encode({ a: new Int32Array([1, 2, 3]) });
    const bad = new Uint8Array(blob);
    // First slab table entry: byteLength field is at table offset +8 (= 28).
    new DataView(bad.buffer).setUint32(28, 0x7fffffff, true);
    expect(() => decode(bad)).toThrow(/slab 0 overruns buffer/);
  });

  it('throws when a slab byteLength is not a multiple of its element size', () => {
    const blob = encode({ a: new Int32Array([1, 2, 3]) });
    const bad = new Uint8Array(blob);
    // Slab 0 is the JSON root; slab 1 is the Int32Array with byteLength=12.
    // The byteLength field of table entry 1 is at offset 20 + 1*12 + 8 = 40.
    // Tamper it to 11.
    new DataView(bad.buffer).setUint32(40, 11, true);
    expect(() => decode(bad)).toThrow(/not a multiple of element size 4/);
  });

  it('throws when rootJsonSlabIndex points to a non-JSON slab', () => {
    const blob = encode({ a: new Int32Array([1, 2]) });
    // Two slabs: [0] = JSON (root), [1] = Int32Array. Point root at slab 1.
    const bad = tamperHeader(blob, 'rootIdx', 1);
    expect(() => decode(bad)).toThrow(/expected SlabType\.Json/);
  });
});
