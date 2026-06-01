mod bench;
mod cli;
mod config;
mod deps;
mod edit;
mod http;
mod index;
mod init;
mod mcp;
mod pathing;
mod read;
mod scanner;
mod search;
mod security;
mod self_update;
mod smart;
mod snapshot;
mod symbols;
mod usage;
mod watcher;

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
            println!("deps files: {}", index.deps.len());
        }
        Command::Snapshot { root } => {
            let index = snapshot::load_or_build(&root)?;
            snapshot::print_info(&index);
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
            level,
            max_bytes,
            hash,
        } => {
            read::read(
                &root,
                &path,
                lines.as_deref(),
                level,
                max_bytes,
                hash.as_deref(),
            )?;
        }
        Command::Smart { path, root } => {
            smart::print(&root, &path)?;
        }
        Command::Gain { root, format } => {
            usage::print_gain(&root, format)?;
        }
        Command::SelfUpdate {
            version,
            install_dir,
        } => {
            self_update::run(&version, install_dir)?;
        }
        Command::Mcp { root } => {
            mcp::serve(&root)?;
        }
        Command::Serve { root, host, port } => {
            http::serve(&root, &host, port)?;
        }
        Command::Outline { path, root } => {
            let index = snapshot::load_or_build(&root)?;
            symbols::print_outline(&index, &path);
        }
        Command::Symbol { name, root, limit } => {
            let index = snapshot::load_or_build(&root)?;
            symbols::print_symbols(&index, &name, limit);
        }
        Command::Deps { path, root } => {
            let index = snapshot::load_or_build(&root)?;
            deps::print_deps(&index, &path);
        }
        Command::Rdeps { path, root } => {
            let index = snapshot::load_or_build(&root)?;
            deps::print_reverse_deps(&index, &path);
        }
        Command::Bench {
            root,
            query,
            symbol,
            limit,
            json,
        } => {
            bench::run(&root, &query, &symbol, limit, json)?;
        }
        Command::Edit {
            path,
            root,
            op,
            start,
            end,
            content,
            hash,
        } => {
            let result = edit::apply(&root, &path, op, start, end, content.as_deref(), &hash)?;
            println!("path: {}", result.path);
            println!("hash: {}", result.hash);
            println!("lines: {}", result.lines);
        }
        Command::Init { target, root } => {
            init::enable(target, &root)?;
        }
        Command::Enable { target, root } => {
            init::enable(target, &root)?;
        }
        Command::Disable { target } => {
            init::disable(target)?;
        }
        Command::McpStatus { target } => {
            init::status(target)?;
        }
        Command::Watch {
            root,
            poll,
            interval_ms,
        } => {
            watcher::watch(&root, poll, interval_ms)?;
        }
        Command::Changes { root } => {
            watcher::print_changes(&root)?;
        }
        Command::Hot { root, limit } => {
            watcher::print_hot(&root, limit)?;
        }
    }

    Ok(())
}
