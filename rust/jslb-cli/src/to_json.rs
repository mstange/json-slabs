use anyhow::{Context, Result};
use json_slabs::format::{SlabTableEntry, SlabType};
use json_slabs::read::ParseError;
use json_writer::{JSONWriter, JSONWriterValue};
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Write};
use std::path::Path;

use crate::skeleton::{self, Ctx, Visitor};
use crate::slab_reader::SlabReader;
use crate::util::{read_full, read_header_and_table};

const DRAIN_THRESHOLD: usize = 64 * 1024;
const READ_BUF: usize = 64 * 1024;

pub fn run(input: &Path, output: Option<&Path>) -> Result<()> {
    let mut file = File::open(input).with_context(|| format!("opening {}", input.display()))?;
    let file_len = file
        .metadata()
        .with_context(|| format!("stat {}", input.display()))?
        .len();

    let (header, slabs) = read_header_and_table(&mut file)?;
    for (i, entry) in slabs.iter().enumerate() {
        entry
            .start_offset
            .checked_add(entry.byte_length)
            .filter(|&e| e <= file_len)
            .ok_or(ParseError::SlabDataOverrun { index: i })?;
    }
    if slabs[header.root_json_slab_index].slab_type != SlabType::Json {
        return Err(ParseError::RootNotJson.into());
    }

    let stdout = io::stdout();
    let mut out: BufWriter<Box<dyn Write>> = match output {
        Some(path) => BufWriter::new(Box::new(
            File::create(path).with_context(|| format!("creating {}", path.display()))?,
        )),
        None => BufWriter::new(Box::new(stdout.lock())),
    };

    let ctx = Ctx {
        file: &file,
        slabs: &slabs,
    };
    {
        let mut emitter = JsonEmitter {
            buffer: String::new(),
            out: &mut out,
        };
        skeleton::walk_json_slab(&ctx, header.root_json_slab_index, &mut emitter)
            .context("resolving slab references")?;
        emitter.flush()?;
    }
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

/// Visitor that streams the resolved JSON to `out`, expanding every
/// placeholder it sees: JSON sub-slabs recurse via the shared
/// skeleton walker; typed-array slabs are expanded inline as a JSON
/// array.
struct JsonEmitter<'a, W: Write> {
    buffer: String,
    out: &'a mut W,
}

impl<W: Write> JsonEmitter<'_, W> {
    fn maybe_drain(&mut self) -> Result<()> {
        if self.buffer.len() >= DRAIN_THRESHOLD {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        self.out.write_all(self.buffer.as_bytes())?;
        self.buffer.clear();
        Ok(())
    }

    fn emit_typed_array(&mut self, ctx: &Ctx<'_>, desc: &SlabTableEntry) -> Result<()> {
        let mut sub = BufReader::with_capacity(READ_BUF, SlabReader::new(ctx.file, desc));

        self.buffer.json_begin_array();
        let mut first = true;
        let elem = desc.slab_type.element_size();
        let mut buf = vec![0u8; (READ_BUF / elem) * elem];

        loop {
            let n = read_full(&mut sub, &mut buf)?;
            if n == 0 {
                break;
            }
            // Slab byte length is a multiple of `elem` (validated in
            // SlabTableEntry::parse), so each fill is elem-aligned.
            debug_assert!(n.is_multiple_of(elem));
            for chunk in buf[..n].chunks_exact(elem) {
                self.buffer.json_begin_array_value(first);
                first = false;
                write_typed_elem(&mut self.buffer, desc.slab_type, chunk);
                self.maybe_drain()?;
            }
        }
        self.buffer.json_end_array(first);
        self.maybe_drain()?;
        Ok(())
    }
}

impl<W: Write> Visitor for JsonEmitter<'_, W> {
    fn enter_object(&mut self) -> Result<()> {
        self.buffer.json_begin_object();
        Ok(())
    }
    fn exit_object(&mut self, empty: bool) -> Result<()> {
        self.buffer.json_end_object(empty);
        self.maybe_drain()
    }
    fn enter_array(&mut self) -> Result<()> {
        self.buffer.json_begin_array();
        Ok(())
    }
    fn exit_array(&mut self, empty: bool) -> Result<()> {
        self.buffer.json_end_array(empty);
        self.maybe_drain()
    }
    fn object_key(&mut self, key: &str, first: bool) -> Result<()> {
        self.buffer.json_object_key(key, first);
        Ok(())
    }
    fn array_element(&mut self, index: usize) -> Result<()> {
        self.buffer.json_begin_array_value(index == 0);
        Ok(())
    }
    fn value_string(&mut self, s: &str) -> Result<()> {
        self.buffer.json_string(s);
        self.maybe_drain()
    }
    fn value_number(&mut self, raw: &str) -> Result<()> {
        // Preserve the exact textual form actson saw — no round-trip
        // through f64 / i64 / u64.
        self.buffer.push_str(raw);
        self.maybe_drain()
    }
    fn value_true(&mut self) -> Result<()> {
        true.write_json(&mut self.buffer);
        self.maybe_drain()
    }
    fn value_false(&mut self) -> Result<()> {
        false.write_json(&mut self.buffer);
        self.maybe_drain()
    }
    fn value_null(&mut self) -> Result<()> {
        self.buffer.json_null();
        self.maybe_drain()
    }
    fn placeholder(&mut self, ctx: &Ctx<'_>, slab_idx: usize) -> Result<()> {
        let slab = ctx
            .slabs
            .get(slab_idx)
            .with_context(|| format!("slab index {slab_idx} out of range"))?;
        if slab.slab_type == SlabType::Json {
            skeleton::walk_json_slab(ctx, slab_idx, self)
        } else {
            self.emit_typed_array(ctx, slab)
        }
    }
}

fn write_typed_elem(buffer: &mut String, slab_type: SlabType, c: &[u8]) {
    match slab_type {
        SlabType::Int8 => (c[0] as i8).write_json(buffer),
        SlabType::Uint8 => c[0].write_json(buffer),
        SlabType::Int16 => i16::from_le_bytes([c[0], c[1]]).write_json(buffer),
        SlabType::Uint16 => u16::from_le_bytes([c[0], c[1]]).write_json(buffer),
        SlabType::Int32 => i32::from_le_bytes([c[0], c[1], c[2], c[3]]).write_json(buffer),
        SlabType::Uint32 => u32::from_le_bytes([c[0], c[1], c[2], c[3]]).write_json(buffer),
        SlabType::Float32 => f32::from_le_bytes([c[0], c[1], c[2], c[3]]).write_json(buffer),
        SlabType::Float64 => {
            f64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]).write_json(buffer)
        }
        SlabType::Int64 => {
            i64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]).write_json(buffer)
        }
        SlabType::Uint64 => {
            u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]).write_json(buffer)
        }
        SlabType::Json => unreachable!("Json slabs handled separately"),
    }
}
