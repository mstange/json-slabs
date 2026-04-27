# JSLB — Binary Container Format

Version 1.

## Overview

A JsonSlabs file is made of the following:

- A fixed-size header: magic `DC DF 4A 53 4C 42 01 00`, version, slab count, root slab index
- A table which lists `(type, offset, byteLength)` for each slab
- The slabs with their data, each at the natural alignment for its type.

A slab is either JSON or typed array data (u8, i32, f64, etc).

All numbers are stored in little-endian.

The root slab is always a JSON slab. It is often the last slab, but doesn't have to be.

JSON slabs embed data from the other slabs via `{ "$s": <slab-index> }` placeholders.

---

## File layout

```
Offset    Size          Description
------    ----          -----------
0         8             Magic bytes: `DC DF 4A 53 4C 42 01 00`
8         4             Version: uint32LE = 1
12        4             Slab count: uint32LE
16        4             Root JSON slab index: uint32LE
20        12 × count    Slab table (see below)
20+12N    …             Slab data (each slab at its natural alignment)
```

---

## Slab table

One 12-byte entry per slab, all fields uint32LE:

```
Offset  Size   Description
------  ----   -----------
0       4      Slab type (see below)
4       4      Absolute start location of the slab's data (multiple of alignment)
8       4      Slab byte length (multiple of alignment)
```

The JSON skeleton is stored as a `TYPE_JSON` slab at index `rootJsonSlabIndex`.
The root may appear at any position in the slab table; decoders locate it via
the `rootJsonSlabIndex`.

All sizes and offsets are uint32LE, which limits the slab size and start offsets to ~4GiB.

---

## Slab types

| Value | TypedArray / Content | Element size | Alignment |
|-------|----------------------|--------------|-----------|
| 0x00  | Int8Array            | 1 byte       | 1         |
| 0x01  | Uint8Array           | 1 byte       | 1         |
| 0x02  | Int16Array           | 2 bytes LE   | 2         |
| 0x03  | Uint16Array          | 2 bytes LE   | 2         |
| 0x04  | Int32Array           | 4 bytes LE   | 4         |
| 0x05  | Uint32Array          | 4 bytes LE   | 4         |
| 0x06  | Float32Array         | 4 bytes LE   | 4         |
| 0x07  | Float64Array         | 8 bytes LE   | 8         |
| 0x08  | BigInt64Array        | 8 bytes LE   | 8         |
| 0x09  | BigUint64Array       | 8 bytes LE   | 8         |
| 0x0a  | UTF-8 JSON bytes     | —            | 1         |

`TYPE_JSON` slabs contain UTF-8–encoded JSON text. They may themselves include
``{ "$s": N }`` placeholders referencing other slabs in the same container,
enabling nested sub-documents without a separate container.

---

## JSON Skeleton

The skeleton is `JSON.stringify` output where TypedArrays have been replaced by
``{ "$s": N }`` objects. `N` is the index of the corresponding data slab.
All other content — strings, nested objects, mixed-type arrays — remains verbatim.

### Reserved shape

An object whose **only** own enumerable key is `$s`, is reserved by the format
as a slab placeholder. Decoders MUST replace it with the referenced slab.

Producers therefore cannot losslessly round-trip user data of this shape
without escaping it. The reference encoder does not escape — applications
that may legitimately contain `{ "$s": <number> }` objects must either avoid
the shape or wrap such values themselves before encoding.
