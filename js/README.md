# json-slabs

Efficient serialization + deserialization of "JSON with binary slabs" for JavaScript
objects which contain typed arrays.

Similarly to `JSON.stringify`, you can call `encode` on a JSON-compatible object.
This will encode the entire object graph and return a `Uint8Array`. If the object
contains typed arrays anywhere in its structure, their contents will be stored in
separate "slabs" within the output buffer. Parsing the buffer with `decode` recreates
the original object structure and places typed arrays in the right spots, simply
re-wrapping the file bytes with appropriate offsets and sizes.

If you keep your object structure simple and store most of your content in typed arrays,
this gets you much higher decode performance than regular JSON, without sacrificing
JSON's flexibility.

## Install

```sh
npm install json-slabs
```

## High-level API

Most users just want `encode` and `decode`:

```ts
import { encode, decode } from 'json-slabs';

// Serialize: object (may contain TypedArrays anywhere) to Uint8Array
const bytes = encode(myObject); // returns Uint8Array

// Deserialize: Uint8Array to object, TypedArrays restored as zero-copy views into `bytes`
const obj = decode<MyType>(bytes);
```

`decode<T>` accepts an optional type parameter to express the expected shape
without a separate cast (but doesn't do any validation).

### Reserved object shape: `{ "$s": <number> }`

An object whose **only** key is `$s` is reserved as a
slab placeholder by the format. The encoder does not detect or
escape user data shaped this way.

## Implementation

Internally, `encode` calls `JSON.stringify` with a replacer function which
detects typed arrays, puts them into separate slabs, and substitutes them with
a placeholder of the shape `{ "$s": N }`.

`decode` calls `JSON.parse` with a reviver function which substitutes the
placeholders with the appropriate typed arrays, wrapping the array buffer of the
parsed `Uint8Array`.

See the [format spec](../FORMAT.md) for the full binary layout.

## Advanced usage

### Avoiding copies during encoding

In addition to `encode`, this library also provides an `encodeToBlob` function.
This is useful when piping to a `CompressionStream` or passing to
`fetch()` / `new Response()`: it avoids allocating one large contiguous buffer
by wrapping the internal chunk list directly in a `Blob`.

### Splitting nested values into their own JSON slabs (`splitOut`)

Both `encode` and `encodeToBlob` accept an optional second argument: a list
of nested values that should each be lifted out of the root JSON into their
own `SlabType.Json` sub-slab. Rules:

1. Matching is by reference identity (`===` / Set membership).
2. Each value must be reachable from `obj`; unreachable entries silently
   have no effect.
3. If the top-level object itself appears in the list, it is ignored — the
   root JSON is never split into a sub-slab.
4. TypedArrays in the list are still encoded as their native typed slab.
   `splitOut` only affects non-TypedArray values.

```ts
import { encode, decode } from 'json-slabs';

const data = { libs: [], shared: { stringArray: ['hello', 'world'] } };
const bytes = encode(data, [data.shared.stringArray]);

// Two JSON slabs in the container:
//   slab 0 (SlabType.Json): ["hello","world"]
//   slab 1 (SlabType.Json, root): {"libs":[],"shared":{"stringArray":{"$s":0}}}
//
// decode(bytes) reconstructs the original object — sub-slab JSON is
// recursively parsed and inlined where the placeholder appeared.
```

This is useful for keeping large or independently-cacheable sub-documents in
their own slabs without dropping to the low-level Builder API.

### Low-level API (Builder)

Use `Builder` when you need finer control — for example, when your encode step
adds codec metadata alongside each slab, or when you want zero-copy streaming
chunks instead of one concatenated buffer:

```ts
import { Builder, decode } from 'json-slabs';

const builder = new Builder();

// Register TypedArrays; get back { "$s": N } placeholder objects
const p1 = builder.addSlab(myInt32Array);
const p2 = builder.addSlab(myFloat64Array);

// Build a JSON skeleton using the placeholders, then finish.
// `toBuffer` accepts either a JSON string or pre-encoded UTF-8 bytes.
const bytes = builder.toBuffer(
  JSON.stringify({ values: p1, weights: p2, label: 'example' }),
);
```

Builder methods:

| Method              | Description                                                                  |
| ------------------- | ---------------------------------------------------------------------------- |
| `addSlab(slab)`     | Register any supported `TypedArray` and return a `{ "$s": N }` placeholder   |
| `addJsonSlab(json)` | Register a nested JSON document (`string` or UTF-8 `Uint8Array`)             |
| `toBuffer(json)`    | Finish and return one concatenated `Uint8Array`                              |
| `toBlob(json)`      | Finish and return a `Blob` (zero-copy from chunks)                           |
| `finish(json)`      | Lower-level: return the container as a list of zero-copy `Uint8Array` chunks |

`addSlab` dispatches by `TypedArray` constructor: `Int8Array`, `Uint8Array`,
`Int16Array`, `Uint16Array`, `Int32Array`, `Uint32Array`, `Float32Array`,
`Float64Array`, `BigInt64Array`, `BigUint64Array`. `Uint8ClampedArray` is
explicitly rejected — copy to a `Uint8Array` first.

`addJsonSlab` registers a nested JSON document as a `SlabType.Json` slab. On
decode, `{ "$s": N }` placeholders pointing to `SlabType.Json` slabs are
recursively JSON-parsed (sharing the same slab index space), enabling lazy or
sub-document nesting.

The Builder enforces single-use: after `finish` / `toBuffer` / `toBlob`, any
further method call throws.

## Exported symbols

| Symbol             | Description                                                                                            |
| ------------------ | ------------------------------------------------------------------------------------------------------ |
| `encode`           | High-level encode: `(obj, splitOut?) => Uint8Array`                                                    |
| `decode<T>`        | High-level decode: `(buffer) => T`                                                                     |
| `encodeToBlob`     | Encode straight to a `Blob` without allocating one contiguous buffer: `(obj, splitOut?) => Blob`       |
| `isJsonSlabsFile`  | Quick magic-byte sniff: `(buffer) => boolean`                                                          |
| `Builder`          | Low-level builder for manual slab construction                                                         |
| `decodeContainer`  | Low-level: parse a blob into `{ slabs, slabTypes, rootJsonSlabIndex }`                                 |
| `jsonSlabBytes`    | `(container, index) => Uint8Array \| null` — returns bytes if the slab is `SlabType.Json`, else `null` |
| `SlabType`         | Const-object with the wire-format type codes (`SlabType.Int8` … `SlabType.Json`); also a type alias for the union of those values |
| `AnySlab`          | Union of all supported TypedArray types                                                                |
| `SlabPlaceholder`  | Type for `{ "$s": N }` placeholder objects                                                             |
| `DecodedContainer` | Return type of `decodeContainer`                                                                       |

## Format

The binary container format is documented in [FORMAT.md](../FORMAT.md) at the
repository root, independent of any language implementation.

## License

MIT.
