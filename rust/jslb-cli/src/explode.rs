use anyhow::{Context, Result};
use json_slabs::format::SlabType;
use json_slabs::read::ParseError;

use std::fs::File;
use std::io::{self, BufWriter};
use std::path::Path;

use crate::slab_reader::SlabReader;
use crate::util::{digits, read_header_and_table};

const COPY_BUF: usize = 64 * 1024;

pub fn run(input: &Path, output_dir: &Path) -> Result<()> {
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

    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("creating directory {}", output_dir.display()))?;

    let pad = digits(slabs.len().saturating_sub(1));

    for (i, entry) in slabs.iter().enumerate() {
        let is_root = i == header.root_json_slab_index;
        let filename = match entry.slab_type {
            SlabType::Json => {
                if is_root {
                    format!("slab-{i:0>pad$}-json-root.json")
                } else {
                    format!("slab-{i:0>pad$}-json.json")
                }
            }
            t => format!("slab-{i:0>pad$}-{}.bin", t.name()),
        };
        let path = output_dir.join(&filename);

        let mut reader = SlabReader::new(&file, entry);
        let out = File::create(&path).with_context(|| format!("writing {}", path.display()))?;
        let mut writer = BufWriter::with_capacity(COPY_BUF, out);
        io::copy(&mut reader, &mut writer)
            .with_context(|| format!("writing {}", path.display()))?;
        writer
            .into_inner()
            .map_err(|e| e.into_error())
            .with_context(|| format!("flushing {}", path.display()))?;

        println!("{filename}  ({} bytes)", entry.byte_length);
    }

    Ok(())
}
