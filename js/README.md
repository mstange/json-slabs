# json-slabs

Efficient serialization + deserialization of "JSON with binary slabs" for JavaScript
objects which contain typed arrays.

Similarly to `JSON.stringify`, you can call `slabify` on a JSON-compatible object.
This will encode the object to a `Uint8Array`. If the object contains typed arrays
anywhere in its structure, their contents will be stored in separate "slabs" within
the output buffer. Parsing the buffer with `parse` recreates the original object
structure and places typed arrays in the right spots, simply re-wrapping the file bytes
with appropriate offsets and sizes.

If you keep your object structure small and store most of your content in typed arrays,
this gets you much higher decode performance than regular JSON, without sacrificing
JSON's flexibility.

## Install

```sh
npm install json-slabs
```

## High-level API

```ts
import { slabify, parse } from 'json-slabs';

// Serialize: object (may contain TypedArrays anywhere) to Uint8Array
const bytes = slabify(myObject); // returns Uint8Array

// Deserialize: Uint8Array to object, TypedArrays restored as zero-copy views into `bytes`
const obj = parse<MyType>(bytes);
```

`parse<T>` accepts an optional type parameter to express the expected shape
without a separate cast (but doesn't do any validation).

### Reserved object shape: `{ "$s": <number> }`

An object whose **only** key is `$s` is reserved as a
slab placeholder by the format. The encoder does not detect or
escape user data shaped this way.

### Decoder requirements

`parse` and `decode` require the input `Uint8Array` to start at an
8-byte-aligned offset within its underlying `ArrayBuffer`. Slab offsets in the
container are relative to the container's byte 0; misalignment would break
zero-copy access to the wider numeric types. The decoder throws an actionable
error if this is violated — copy to a fresh `Uint8Array` first.

## Implementation

Internally, `slabify` calls `JSON.stringify` with a replacer function which
detects typed arrays, puts them into separate slabs, and substitutes them with
a placeholder of the shape `{ "$s": N }`.

`parse` calls `JSON.parse` with a reviver function which substitutes the
placeholders with the appropriate typed arrays, wrapping the array buffer of the
parsed `Uint8Array`.

See the [format spec](../FORMAT.md) for the full binary layout.

## Advanced usage

### Avoiding copies during encoding

In addition to `slabify`, this library also provides a `slabifyToBlob` function.
This is useful when piping to a `CompressionStream` or passing to
`fetch()` / `new Response()`: it avoids allocating one large contiguous buffer
by wrapping the internal chunk list directly in a `Blob`.

### Splitting nested values into their own JSON slabs

Both `slabify` and `slabifyToBlob` accept an optional second argument: a list
of nested values that should each be lifted out of the root JSON into their
own TYPE_JSON sub-slab. Matching is by reference identity. If the top-level
object itself appears in the list, it is ignored — the root JSON is never
split into a sub-slab.

```ts
import { slabify, parse } from 'json-slabs';

const data = { libs: [], shared: { stringArray: ['hello', 'world'] } };
const bytes = slabify(data, [data.shared.stringArray]);

// Two JSON slabs in the container:
//   slab 0 (TYPE_JSON): ["hello","world"]
//   slab 1 (TYPE_JSON, root): {"libs":[],"shared":{"stringArray":{"$s":0}}}
//
// parse(bytes) reconstructs the original object — sub-slab JSON is
// recursively parsed and inlined where the placeholder appeared.
```

This is useful for keeping large or independently-cacheable sub-documents in
their own slabs without dropping to the low-level Builder API.

### Low-level API (Builder)

Use `Builder` when you need finer control — for example, when your encode step
adds codec metadata alongside each slab, or when you want zero-copy streaming
chunks instead of one concatenated buffer:

```ts
import { Builder, parse } from 'json-slabs';

const builder = new Builder();

// Register TypedArrays; get back { "$s": N } placeholder objects
const p1 = builder.addI32Slab(myInt32Array);
const p2 = builder.addF64Slab(myFloat64Array);

// Build a JSON skeleton using the placeholders
const skeleton = { values: p1, weights: p2, label: 'example' };
const jsonBytes = new TextEncoder().encode(JSON.stringify(skeleton));

// Finish: appends JSON as the root slab, returns zero-copy Uint8Array chunks
const chunks = builder.finish(jsonBytes);
```

Builder methods for all supported types:

| Method                   | TypedArray             |
| ------------------------ | ---------------------- |
| `addI8Slab(slab)`        | Int8Array              |
| `addU8Slab(slab)`        | Uint8Array             |
| `addI16Slab(slab)`       | Int16Array             |
| `addU16Slab(slab)`       | Uint16Array            |
| `addI32Slab(slab)`       | Int32Array             |
| `addU32Slab(slab)`       | Uint32Array            |
| `addF32Slab(slab)`       | Float32Array           |
| `addF64Slab(slab)`       | Float64Array           |
| `addI64Slab(slab)`       | BigInt64Array          |
| `addU64Slab(slab)`       | BigUint64Array         |
| `addJsonSlab(jsonBytes)` | UTF-8 JSON (TYPE_JSON) |

`addJsonSlab` registers a nested JSON document (UTF-8 bytes) as a TYPE_JSON
slab. On parse, `{ "$s": N }` placeholders pointing to TYPE_JSON slabs are
recursively JSON-parsed (sharing the same slab index space), enabling lazy or
sub-document nesting.

## Exported symbols

| Symbol                              | Description                                                                       |
| ----------------------------------- | --------------------------------------------------------------------------------- |
| `slabify`, `slabifyToBlob`, `parse` | High-level encode / decode                                                        |
| `isJsonSlabsFile`                   | Quick magic-byte sniff: `(buffer: Uint8Array) => boolean`                         |
| `Builder`                           | Low-level builder for manual slab construction                                    |
| `decode`                            | Low-level: parse a blob into `{ jsonBytes, slabs, slabTypes, rootJsonSlabIndex }` |
| `AnySlab`                           | Union of all supported TypedArray types                                           |
| `SlabPlaceholder`                   | Type for `{ "$s": N }` placeholder objects                                        |
| `DecodedContainer`                  | Return type of `decode`                                                           |
| `TYPE_*` constants                  | Type values for each slab kind (`TYPE_INT8` … `TYPE_JSON`)                        |

## Format

The binary container format is documented in [FORMAT.md](../FORMAT.md) at the
repository root, independent of any language implementation.

## License

MIT.
