use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod explode;
mod slab_reader;
mod util;

#[derive(Parser)]
#[command(name = "jslb", about = "JsonSlab (.jslb) file utilities")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Unpack a .jslb file into a directory, one file per slab
    Explode { input: PathBuf, output_dir: PathBuf },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Explode { input, output_dir } => explode::run(&input, &output_dir),
    }
}
