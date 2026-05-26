mod cli;
mod config;
mod index;
mod mcp;
mod pathing;
mod read;
mod scanner;
mod search;
mod security;
mod snapshot;
mod symbols;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Index { root } => {
            let index = index::ProjectIndex::build(&root)?;
            snapshot::save(&index)?;
            println!("indexed {} files", index.files.len());
        }
        Command::Tree { root } => {
            let index = snapshot::load_or_build(&root)?;
            for file in index.files {
                println!("{}\t{} lines\t{} bytes", file.path, file.lines, file.bytes);
            }
        }
        Command::Status { root } => {
            let index = snapshot::load_or_build(&root)?;
            println!("root: {}", index.root);
            println!("files: {}", index.files.len());
            println!("words: {}", index.words.len());
            println!("trigrams: {}", index.trigrams.len());
            println!("symbols: {}", index.symbols.len());
            println!("symbol names: {}", index.symbols_by_name.len());
        }
        Command::Search { query, root, limit } => {
            let index = snapshot::load_or_build(&root)?;
            search::search(&index, &query, limit)?;
        }
        Command::Word { word, root, limit } => {
            let index = snapshot::load_or_build(&root)?;
            search::word(&index, &word, limit);
        }
        Command::Read {
            path,
            root,
            lines,
            max_bytes,
            hash,
        } => {
            read::read(&root, &path, lines.as_deref(), max_bytes, hash.as_deref())?;
        }
        Command::Mcp { root } => {
            mcp::serve(&root)?;
        }
        Command::Outline { path, root } => {
            let index = snapshot::load_or_build(&root)?;
            symbols::print_outline(&index, &path);
        }
        Command::Symbol { name, root, limit } => {
            let index = snapshot::load_or_build(&root)?;
            symbols::print_symbols(&index, &name, limit);
        }
    }

    Ok(())
}
