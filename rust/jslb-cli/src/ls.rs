use anyhow::{Context, Result};
use json_slabs::format::{Header, SlabTableEntry, SlabType};
use json_slabs::read::ParseError;
use json_writer::JSONWriter;
use std::fs::File;
use std::path::Path;

use crate::skeleton::{self, Ctx, Visitor};
use crate::util::{digits, read_header_and_table};

pub fn run(input: &Path, no_paths: bool) -> Result<()> {
    let mut file = File::open(input).with_context(|| format!("opening {}", input.display()))?;
    let (header, entries) = read_header_and_table(&mut file)?;

    let paths = if no_paths {
        None
    } else {
        let file_len = file
            .metadata()
            .with_context(|| format!("stat {}", input.display()))?
            .len();
        for (i, entry) in entries.iter().enumerate() {
            entry
                .start_offset
                .checked_add(entry.byte_length)
                .filter(|&e| e <= file_len)
                .ok_or(ParseError::SlabDataOverrun { index: i })?;
        }
        if entries[header.root_json_slab_index].slab_type != SlabType::Json {
            return Err(ParseError::RootNotJson.into());
        }
        Some(resolve_paths(&file, &entries, header.root_json_slab_index)?)
    };

    print_table(&header, &entries, paths.as_deref());
    Ok(())
}

fn resolve_paths(
    file: &File,
    entries: &[SlabTableEntry],
    root: usize,
) -> Result<Vec<Option<String>>> {
    let mut finder = PathFinder::new(entries.len(), root);
    let ctx = Ctx {
        file,
        slabs: entries,
    };
    skeleton::walk_json_slab(&ctx, root, &mut finder).context("walking JSON skeletons")?;
    Ok(finder.paths)
}

/// Visitor that records, for every slab, the first jq-style path
/// that points at it. Maintains an explicit path stack: each
/// `object_key` / `array_element` push appends one component to
/// `cur_path` and records its byte length so `exit_*` and the next
/// sibling can pop it again.
struct PathFinder {
    paths: Vec<Option<String>>,
    cur_path: String,
    component_lens: Vec<usize>,
}

impl PathFinder {
    fn new(slab_count: usize, root: usize) -> Self {
        let mut paths = vec![None; slab_count];
        paths[root] = Some(".".to_string());
        Self {
            paths,
            // Starts empty: object_key("frame", true) pushes ".frame",
            // so a placeholder at `.frame` records exactly that. The
            // root's `.` lives only in `paths[root]` above.
            cur_path: String::new(),
            component_lens: Vec::new(),
        }
    }

    fn push(&mut self, component: &str) {
        self.cur_path.push_str(component);
        self.component_lens.push(component.len());
    }

    fn pop(&mut self) {
        let len = self.component_lens.pop().expect("balanced push/pop");
        self.cur_path.truncate(self.cur_path.len() - len);
    }
}

impl Visitor for PathFinder {
    fn object_key(&mut self, key: &str, first: bool) -> Result<()> {
        if !first {
            self.pop();
        }
        let component = format_key(key);
        self.push(&component);
        Ok(())
    }
    fn exit_object(&mut self, empty: bool) -> Result<()> {
        if !empty {
            self.pop();
        }
        Ok(())
    }
    fn array_element(&mut self, index: usize) -> Result<()> {
        if index > 0 {
            self.pop();
        }
        self.push(&format!("[{index}]"));
        Ok(())
    }
    fn exit_array(&mut self, empty: bool) -> Result<()> {
        if !empty {
            self.pop();
        }
        Ok(())
    }
    fn placeholder(&mut self, ctx: &Ctx<'_>, slab_idx: usize) -> Result<()> {
        let already_seen = self.paths.get(slab_idx).is_some_and(|p| p.is_some());
        if let Some(slot) = self.paths.get_mut(slab_idx) {
            if slot.is_none() {
                *slot = Some(if self.cur_path.is_empty() {
                    ".".to_string()
                } else {
                    self.cur_path.clone()
                });
            }
        }
        // Recurse only the first time we see a slab — that captures
        // every path it reaches, and the guard breaks self-cycles in
        // malformed files.
        if !already_seen {
            if let Some(slab) = ctx.slabs.get(slab_idx) {
                if slab.slab_type == SlabType::Json {
                    skeleton::walk_json_slab(ctx, slab_idx, self)?;
                }
            }
        }
        Ok(())
    }
}

/// Format an object key as a jq-style path component: `.foo` for
/// simple identifiers, `["..."]` (JSON-escaped) for anything else.
fn format_key(k: &str) -> String {
    if is_simple_identifier(k) {
        format!(".{k}")
    } else {
        let mut buf = String::with_capacity(k.len() + 4);
        buf.push('[');
        buf.json_string(k);
        buf.push(']');
        buf
    }
}

/// Determines whether we can use `.key` when printing this JSON key.
/// (If false we have to use `["key"]`.)
fn is_simple_identifier(k: &str) -> bool {
    let mut chars = k.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn print_table(header: &Header, entries: &[SlabTableEntry], paths: Option<&[Option<String>]>) {
    const UNREFERENCED: &str = "(unreferenced)";

    let byte_len = |e: &SlabTableEntry| e.byte_length as usize;
    let elem_count = |e: &SlabTableEntry| byte_len(e) / e.slab_type.element_size();
    let fmt_bytes = |n: usize| humansize::format_size(n as u64, humansize::DECIMAL);

    let total_bytes: usize = entries.iter().map(byte_len).sum();

    let idx_w = digits(entries.len().saturating_sub(1)).max(3);
    let bytes_w = entries
        .iter()
        .map(|e| fmt_bytes(byte_len(e)).len())
        .max()
        .unwrap_or(0)
        .max(fmt_bytes(total_bytes).len())
        .max(5);
    let elems_w = entries
        .iter()
        .filter(|e| e.slab_type != SlabType::Json)
        .map(|e| digits(elem_count(e)))
        .max()
        .unwrap_or(0)
        .max(8);

    let path_w = paths.map(|ps| {
        let max_resolved = ps
            .iter()
            .filter_map(|p| p.as_ref())
            .map(|s| s.len())
            .max()
            .unwrap_or(0);
        let unref_w = if ps.iter().any(|p| p.is_none()) {
            UNREFERENCED.len()
        } else {
            0
        };
        max_resolved.max(unref_w).max(4)
    });

    let print_rule = || match path_w {
        Some(pw) => {
            println!(
                "{0:-<idx_w$}  {0:-<7}  {0:->bytes_w$}  {0:->elems_w$}  {0:-<pw$}",
                ""
            )
        }
        None => println!("{0:-<idx_w$}  {0:-<7}  {0:->bytes_w$}  {0:->elems_w$}", ""),
    };

    match path_w {
        Some(pw) => println!(
            "{:>idx_w$}  {:<7}  {:>bytes_w$}  {:>elems_w$}  {:<pw$}",
            "idx", "type", "bytes", "elements", "path"
        ),
        None => println!(
            "{:>idx_w$}  {:<7}  {:>bytes_w$}  {:>elems_w$}",
            "idx", "type", "bytes", "elements"
        ),
    }
    print_rule();

    for (i, entry) in entries.iter().enumerate() {
        let is_root = i == header.root_json_slab_index;
        let elems = if entry.slab_type == SlabType::Json {
            "-".to_string()
        } else {
            elem_count(entry).to_string()
        };
        let notes = if is_root { "  (root)" } else { "" };
        match (path_w, paths) {
            (Some(pw), Some(ps)) => {
                let path = ps[i].as_deref().unwrap_or(UNREFERENCED);
                println!(
                    "{i:>idx_w$}  {:<7}  {:>bytes_w$}  {:>elems_w$}  {:<pw$}{notes}",
                    entry.slab_type.name(),
                    fmt_bytes(byte_len(entry)),
                    elems,
                    path,
                );
            }
            _ => {
                println!(
                    "{i:>idx_w$}  {:<7}  {:>bytes_w$}  {:>elems_w$}{notes}",
                    entry.slab_type.name(),
                    fmt_bytes(byte_len(entry)),
                    elems,
                );
            }
        }
    }

    print_rule();
    let total_label = format!(
        "{} slab{}",
        entries.len(),
        if entries.len() == 1 { "" } else { "s" }
    );
    println!(
        "{:>idx_w$}  {:<7}  {:>bytes_w$}",
        "",
        total_label,
        fmt_bytes(total_bytes)
    );
}
