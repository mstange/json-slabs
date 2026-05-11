# json-slabs

Efficient serialization + deserialization of "JSON with binary slabs" for JavaScript
objects which contain typed arrays.

`encode(obj)` will return a `Uint8Array` with the JSLB file bytes. If the object
contains typed arrays anywhere in its structure, their contents will be embedded
verbatim within the returned buffer, as "slabs" whose location is indicated in the
file header.

`decode(buffer)` recreates the original object structure, with typed arrays restored
in the right spots. These typed arrays share the buffer's underlying `ArrayBuffer`.

This is much more efficient than `JSON.stringify` and `JSON.parse` in cases where
the typed array contents are a major contributor to the overall file size. And
unlike regular JSON, typed arrays preserve their typed-array-ness and don't just
become regular arrays of numbers.

`Uint8ClampedArray` becomes a regular `Uint8Array` - the file format
treats them the same.

## Install

```sh
npm install json-slabs
```

## High-level API

You'll usually just need `encode` and `decode`:

```ts
import { encode, decode } from 'json-slabs';

// Serialize: object (may contain TypedArrays anywhere) to Uint8Array
const bytes = encode(myObject); // returns Uint8Array

// Deserialize: Uint8Array to object, TypedArrays restored as zero-copy views into `bytes`
const obj = decode<MyType>(bytes);
```

`decode<T>` accepts an optional type parameter to express the expected shape
without a separate cast (but doesn't do any validation).

### Supported object shapes

Supports any object that works with `JSON.stringify`, except for objects which
match the internal "slab placeholder" object shape: `{ "$s": <number> }`:

An object whose **only** key is `$s` is reserved as a
slab placeholder by the format. The encoder does not detect or
escape user data shaped this way.

## Implementation

Internally, `encode` calls `JSON.stringify` with a replacer function which
detects typed arrays, puts them into separate slabs, and substitutes them with
a placeholder of the shape `{ "$s": N }`. The string from the `stringify` is
converted to a `Uint8Array` and becomes the "root slab". The slab bytes are then
arranged as described by the container file format, with a header and a slab table.

`decode` calls `JSON.parse` with a reviver function which substitutes the
placeholders with the appropriate typed arrays, which are created around the same
array buffer as the parsed `Uint8Array`.

See the [format spec](../FORMAT.md) for the full binary layout.

## Advanced usage

### Avoiding copies during encoding

In addition to `encode`, this library also provides an `encodeToBlob` function.
This is more efficient when piping to a `CompressionStream` or passing to
`fetch()` / `new Response()`: the `Blob` is created with a list of individual buffers
rather than around one large contiguous buffer, and some of fragment buffers are
just views of the original typed array buffer contents.

### Splitting nested values into their own JSON slabs (`splitOut`)

Both `encode` and `encodeToBlob` accept an optional second argument: a list
of nested values that should each be lifted out of the root JSON into their
own `SlabType.Json` sub-slab.

Matching is done by reference identity (`===`).

(Don't put TypedArrays into the `splitOut` list. They'll be encoded as their
native typed slab either way. `splitOut` only affects non-TypedArray values.)

Example:

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
their own slabs.

Here are same cases when you may want to use this abilitiy:

1. If you have cases where you only want to look at the root JSON slab -
   JSON parsing of the root slab faster if there's less data in the root slab.
2. If you run into maximum string size limits
3. If you want to avoid large contiguous allocations during parsing

### Low-level API (Builder)

Use `Builder` when you need finer control:

```ts
import { Builder } from 'json-slabs';

const builder = new Builder();

// Register TypedArrays.
const p1 = builder.addSlab(myInt32Array);   // returns { "$s": N } placeholder object
const p2 = builder.addSlab(myFloat64Array); // same

// Stringify skeleton JSON.
const s = JSON.stringify({ values: p1, weights: p2, label: 'example' });

// Assemeble file bytes.
const bytes = builder.toBuffer(s); // can also be called with a Uint8Array
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
accepted and stored as a `Uint8Array` slab; it decodes back as `Uint8Array`.

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
| `SlabType`         | Const-object with the wire-format type codes (`SlabType.Int8` … `SlabType.Json`); also a type alias for the union of those values |
| `AnySlab`          | Union of all supported TypedArray types                                                                |
| `SlabPlaceholder`  | Type for `{ "$s": N }` placeholder objects                                                             |
| `DecodedContainer` | Return type of `decodeContainer`                                                                       |

## Format

The binary container format is documented in [FORMAT.md](../FORMAT.md) at the
repository root, independent of any language implementation.

## License

MIT.
