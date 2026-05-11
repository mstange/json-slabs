# JsonSlabs (JSLB)

A binary container format for JSON-shaped data with embedded typed arrays.
The typed array data is stored out-of-line as bytes. This means that they
can be arbitrarily long without affecting parsing performance of the object
structure.

## JavaScript library

The JS library provides `encode` and `decode` functions which work similarly
to `JSON.stringify` and `JSON.parse` (but the encoding is a `Uint8Array` and
not a string):

```js
import { encode, decode } from 'json-slabs';
const bytes = encode({ data: new Float64Array([1.5, ..., 24.0]) });
const obj = decode(bytes);
```

The `decode` call is fast because it doesn't have to walk the `Float64Array`
bytes - internally it's just a `JSON.parse` of `'{"data":{"$s":0}}'`
(the "skeleton JSON") and a substitution of the placeholder object with the typed
array. It creates a `Float64Array` as a view of the underlying `ArrayBuffer`
with no copying of array data.

See [js/README.md](js/README.md) for the full API.

## Format

A JsonSlabs file is made of the following:

- A fixed-size header: magic `DC DF 4A 53 4C 42 01 00`, version, slab count, root slab index
- A table which lists `(type, offset, byteLength)` for each slab
- The slabs with their data, each at the natural alignment for its type.

A slab is either JSON or typed array data (u8, i32, f64, etc). The alignment allows
typed arrays to be created as views into the file buffer.

JSON slabs embed data from the other slabs via `{ "$s": <slab-index> }` placeholders.

See [FORMAT.md](FORMAT.md) for details.

## Motivation

This format was created to solve a problem in the [Firefox profiler](https://github.com/firefox-devtools/profiler): Our profile files took too long to parse.

Profile files have the following properties:

- Structured data with a versioned JSON shape that we make frequent changes to
- The majority of the JSON bytes were encoding long arrays of numbers
- Data tables were already organized in a columnar layout
- We have use cases where we want to quickly read just a particular small piece
  of the JSON data (specifically, `samply load` just needs to read `profile.libs`)

None of the other binary container formats we looked at were quite what we wanted.

JsonSlabs hits the following points:

- Schema-less / self-describing object structure content, just like JSON (so that we
  don't have to generate a new parser every time we make a change to the shape) -
  unlike schema-based formats like flatbuffers
- Simple to parse / small dependency, mostly taking advantage of the existing
  `JSON.parse` in JS engines
- Zero-copy typed arrays - unlike compact streaming formats like Protobuf
- O(1) access of the root slab thanks to the slab table at the top of the file -
  avoids having to iterate over all the slabs to find the one that contains the
  `profile.libs` data

Something like flexbuffers would probably have worked for our use case, but would be
a bigger dependency and a more complicated format. Flexbuffers also currently
[doesn't](https://github.com/google/flatbuffers/issues/8450) appear to support zero-copy typed array mapping in its JS implementation.

## License

MIT. See [LICENSE](LICENSE).
