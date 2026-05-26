use std::path::PathBuf;

use camino::Utf8PathBuf;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "repolens")]
#[command(about = "Fast local codebase index for agents")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    Index {
        #[arg(default_value = ".")]
        root: PathBuf,
    },
    Tree {
        #[arg(default_value = ".")]
        root: PathBuf,
    },
    Status {
        #[arg(default_value = ".")]
        root: PathBuf,
    },
    Search {
        query: String,
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
    Word {
        word: String,
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
    Read {
        path: Utf8PathBuf,
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(long)]
        lines: Option<String>,
        #[arg(long)]
        max_bytes: Option<usize>,
        #[arg(long)]
        hash: Option<String>,
    },
    Mcp {
        #[arg(default_value = ".")]
        root: PathBuf,
    },
    Outline {
        path: Utf8PathBuf,
        #[arg(default_value = ".")]
        root: PathBuf,
    },
    Symbol {
        name: String,
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
}
