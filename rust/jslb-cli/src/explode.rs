use anyhow::{Context, Result};
use json_slabs::{ParseError, SlabDirectory, SlabType};

use std::fs::File;
use std::io::{self, BufWriter};
use std::path::Path;

use crate::slab_reader::SlabReader;
use crate::util::digits;

const COPY_BUF: usize = 64 * 1024;

pub fn run(input: &Path, output_dir: &Path) -> Result<()> {
    let mut file = File::open(input).with_context(|| format!("opening {}", input.display()))?;
    let file_len = file
        .metadata()
        .with_context(|| format!("stat {}", input.display()))?
        .len();

    let dir = SlabDirectory::read(&mut file).context("reading JSLB header and slab table")?;
    dir.validate_extents(file_len)?;
    if dir.root_entry().slab_type != SlabType::Json {
        return Err(ParseError::RootNotJson.into());
    }

    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("creating directory {}", output_dir.display()))?;

    let pad = digits(dir.entries.len().saturating_sub(1));
    let root_json_index = dir.root_json_index();

    for (i, entry) in dir.entries.iter().enumerate() {
        let is_root = i == root_json_index;
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
