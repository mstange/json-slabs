use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod explode;
mod ls;
mod skeleton;
mod slab_reader;
mod to_json;
mod util;

#[derive(Parser)]
#[command(name = "jslb", about = "JsonSlab (.jslb) file utilities")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Convert a .jslb file to JSON, resolving all slab references
    ToJson {
        input: PathBuf,
        /// Output path (default: print to stdout)
        output: Option<PathBuf>,
    },
    /// Unpack a .jslb file into a directory, one file per slab
    Explode { input: PathBuf, output_dir: PathBuf },
    /// Print a size breakdown of all slabs, with the first JSON path
    /// that reaches each slab.
    Ls {
        input: PathBuf,
        /// Skip JSON parsing; just decode the slab table.
        #[arg(long)]
        no_paths: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::ToJson { input, output } => to_json::run(&input, output.as_deref()),
        Command::Explode { input, output_dir } => explode::run(&input, &output_dir),
        Command::Ls { input, no_paths } => ls::run(&input, no_paths),
    }
}
