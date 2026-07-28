//! Streaming walker for the JSON skeleton of a `.jslb` file. Reads
//! one JSON-typed slab at a time via [`SlabReader`], drives an
//! [`actson::JsonParser`] over its bytes, and dispatches each JSON
//! event to a [`Visitor`].
//!
//! The walker transparently detects the placeholder shape
//! `{"$s": <non-negative integer>}` with a three-token lookahead. When
//! it sees a real placeholder, the visitor gets a single
//! [`Visitor::placeholder`] call. When the lookahead reveals a regular
//! object that happens to start with `$s`, the walker replays the
//! tokens as `enter_object` / `object_key("$s")` / `value_number(...)`
//! etc. so the visitor never has to know that the lookahead happened.
//!
//! Visitors decide for themselves whether to recurse into a JSON
//! sub-slab by calling [`walk_json_slab`] again from `placeholder`.

use actson::feeder::BufReaderJsonFeeder;
use actson::{JsonEvent, JsonParser};
use anyhow::{Context, Result, anyhow, bail};
use json_slabs::{SLAB_REF_KEY, SlabTableEntry, SlabType};
use std::fs::File;
use std::io::BufReader;

use crate::slab_reader::SlabReader;

const READ_BUF: usize = 64 * 1024;

/// File handle + slab table threaded through the walker so visitors
/// can look slabs up by index (typed-array bytes, JSON sub-slab
/// recursion).
pub struct Ctx<'a> {
    pub file: &'a File,
    pub slabs: &'a [SlabTableEntry],
}

/// Sink for skeleton events. The walker calls these in document order
/// as it parses a JSON slab. Default impls are no-ops; override only
/// the methods you care about. `first`/`index` describe position
/// within the enclosing container.
pub trait Visitor {
    fn enter_object(&mut self) -> Result<()> {
        Ok(())
    }
    fn exit_object(&mut self, empty: bool) -> Result<()> {
        let _ = empty;
        Ok(())
    }
    fn enter_array(&mut self) -> Result<()> {
        Ok(())
    }
    fn exit_array(&mut self, empty: bool) -> Result<()> {
        let _ = empty;
        Ok(())
    }
    fn object_key(&mut self, key: &str, first: bool) -> Result<()> {
        let _ = (key, first);
        Ok(())
    }
    fn array_element(&mut self, index: usize) -> Result<()> {
        let _ = index;
        Ok(())
    }
    fn value_string(&mut self, s: &str) -> Result<()> {
        let _ = s;
        Ok(())
    }
    /// Numeric literal in its original textual form (no `f64` /
    /// `i64` round-trip), so visitors that re-emit JSON preserve the
    /// exact spelling.
    fn value_number(&mut self, raw: &str) -> Result<()> {
        let _ = raw;
        Ok(())
    }
    fn value_true(&mut self) -> Result<()> {
        Ok(())
    }
    fn value_false(&mut self) -> Result<()> {
        Ok(())
    }
    fn value_null(&mut self) -> Result<()> {
        Ok(())
    }
    /// Fired for each `{"$s": N}` placeholder. `slab_idx` is unchecked
    /// against the slab table; the visitor must bounds-check it if it
    /// cares.
    fn placeholder(&mut self, ctx: &Ctx<'_>, slab_idx: usize) -> Result<()> {
        let _ = (ctx, slab_idx);
        Ok(())
    }
}

type SkeletonParser<'a> = JsonParser<BufReaderJsonFeeder<SlabReader<'a>>>;

/// Walk a JSON-typed slab, dispatching events to `v`. The slab must
/// have type [`SlabType::Json`].
pub fn walk_json_slab<V: Visitor>(ctx: &Ctx<'_>, slab_idx: usize, v: &mut V) -> Result<()> {
    let desc = &ctx.slabs[slab_idx];
    debug_assert_eq!(desc.slab_type, SlabType::Json);

    let reader = BufReader::with_capacity(READ_BUF, SlabReader::new(ctx.file, desc));
    let feeder = BufReaderJsonFeeder::new(reader);
    let mut parser = JsonParser::new(feeder);

    walk_value(&mut parser, ctx, v)?;

    if let Some(extra) = next_real_event(&mut parser)? {
        bail!("unexpected trailing event in skeleton: {extra:?}");
    }
    Ok(())
}

fn next_real_event(parser: &mut SkeletonParser) -> Result<Option<JsonEvent>> {
    loop {
        match parser
            .next_event()
            .map_err(|e| anyhow!("parsing JSON skeleton: {e}"))?
        {
            Some(JsonEvent::NeedMoreInput) => parser
                .feeder
                .fill_buf()
                .map_err(|e| anyhow!("reading JSON skeleton: {e}"))?,
            other => return Ok(other),
        }
    }
}

fn walk_value<V: Visitor>(parser: &mut SkeletonParser, ctx: &Ctx<'_>, v: &mut V) -> Result<()> {
    let event =
        next_real_event(parser)?.ok_or_else(|| anyhow!("unexpected end of JSON skeleton"))?;
    walk_event(parser, event, ctx, v)
}

fn walk_event<V: Visitor>(
    parser: &mut SkeletonParser,
    event: JsonEvent,
    ctx: &Ctx<'_>,
    v: &mut V,
) -> Result<()> {
    match event {
        JsonEvent::StartObject => walk_object(parser, ctx, v),
        JsonEvent::StartArray => walk_array(parser, ctx, v),
        JsonEvent::ValueString => {
            let s = parser.current_str().context("reading string value")?;
            v.value_string(s)
        }
        JsonEvent::ValueInt | JsonEvent::ValueFloat => {
            let s = parser.current_str().context("reading number value")?;
            v.value_number(s)
        }
        JsonEvent::ValueTrue => v.value_true(),
        JsonEvent::ValueFalse => v.value_false(),
        JsonEvent::ValueNull => v.value_null(),
        other => bail!("unexpected event in value position: {other:?}"),
    }
}

fn walk_object<V: Visitor>(parser: &mut SkeletonParser, ctx: &Ctx<'_>, v: &mut V) -> Result<()> {
    let event = next_real_event(parser)?
        .ok_or_else(|| anyhow!("unexpected end of JSON skeleton inside object"))?;
    match event {
        JsonEvent::EndObject => {
            v.enter_object()?;
            v.exit_object(true)
        }
        JsonEvent::FieldName => {
            let key = parser.current_str().context("reading object key")?;
            if key == SLAB_REF_KEY {
                try_placeholder(parser, ctx, v)
            } else {
                v.enter_object()?;
                v.object_key(key, true)?;
                walk_value(parser, ctx, v)?;
                continue_object(parser, ctx, v, false)
            }
        }
        other => bail!("unexpected event after StartObject: {other:?}"),
    }
}

/// We've seen `StartObject` then `FieldName("$s")`. Decide whether
/// this is a placeholder or a regular object that happens to start
/// with `$s`.
fn try_placeholder<V: Visitor>(
    parser: &mut SkeletonParser,
    ctx: &Ctx<'_>,
    v: &mut V,
) -> Result<()> {
    let value_event = next_real_event(parser)?
        .ok_or_else(|| anyhow!("unexpected end of JSON skeleton after $s key"))?;

    let candidate_idx: Option<usize> = if value_event == JsonEvent::ValueInt {
        parser
            .current_str()
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
    } else {
        None
    };

    let Some(idx) = candidate_idx else {
        // Value isn't a non-negative integer literal — not a
        // placeholder. Emit `{"$s": <value>` and continue the object.
        v.enter_object()?;
        v.object_key(SLAB_REF_KEY, true)?;
        walk_event(parser, value_event, ctx, v)?;
        return continue_object(parser, ctx, v, false);
    };

    // Save the integer text before the next event invalidates the
    // parser's current_str buffer — we may need to re-emit it.
    let idx_text = parser
        .current_str()
        .context("reading $s integer text")?
        .to_owned();

    let third = next_real_event(parser)?
        .ok_or_else(|| anyhow!("unexpected end of JSON skeleton after $s value"))?;

    match third {
        JsonEvent::EndObject => v.placeholder(ctx, idx),
        JsonEvent::FieldName => {
            // `{"$s": N, ...}` — regular object with $s as one member.
            v.enter_object()?;
            v.object_key(SLAB_REF_KEY, true)?;
            v.value_number(&idx_text)?;
            let key = parser.current_str().context("reading object key")?;
            v.object_key(key, false)?;
            walk_value(parser, ctx, v)?;
            continue_object(parser, ctx, v, false)
        }
        other => bail!("unexpected event after $s value: {other:?}"),
    }
}

fn continue_object<V: Visitor>(
    parser: &mut SkeletonParser,
    ctx: &Ctx<'_>,
    v: &mut V,
    mut first: bool,
) -> Result<()> {
    loop {
        let event = next_real_event(parser)?
            .ok_or_else(|| anyhow!("unexpected end of JSON skeleton inside object"))?;
        match event {
            JsonEvent::EndObject => return v.exit_object(false),
            JsonEvent::FieldName => {
                let key = parser.current_str().context("reading object key")?;
                v.object_key(key, first)?;
                first = false;
                walk_value(parser, ctx, v)?;
            }
            other => bail!("unexpected event inside object: {other:?}"),
        }
    }
}

fn walk_array<V: Visitor>(parser: &mut SkeletonParser, ctx: &Ctx<'_>, v: &mut V) -> Result<()> {
    v.enter_array()?;
    let mut index = 0usize;
    loop {
        let event = next_real_event(parser)?
            .ok_or_else(|| anyhow!("unexpected end of JSON skeleton inside array"))?;
        if event == JsonEvent::EndArray {
            return v.exit_array(index == 0);
        }
        v.array_element(index)?;
        index += 1;
        walk_event(parser, event, ctx, v)?;
    }
}
