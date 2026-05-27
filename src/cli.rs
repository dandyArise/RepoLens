use std::path::PathBuf;

use camino::Utf8PathBuf;
use clap::{Parser, Subcommand, ValueEnum};

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
    Snapshot {
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
    Deps {
        path: Utf8PathBuf,
        #[arg(default_value = ".")]
        root: PathBuf,
    },
    Rdeps {
        path: Utf8PathBuf,
        #[arg(default_value = ".")]
        root: PathBuf,
    },
    Bench {
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(long, default_value = "ProjectIndex")]
        query: String,
        #[arg(long, default_value = "ProjectIndex")]
        symbol: String,
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
    Edit {
        path: Utf8PathBuf,
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(long, value_enum)]
        op: EditOpArg,
        #[arg(long)]
        start: usize,
        #[arg(long)]
        end: Option<usize>,
        #[arg(long)]
        content: Option<String>,
        #[arg(long)]
        hash: String,
    },
    Init {
        #[arg(long, value_enum, default_value_t = InitTargetArg::All)]
        target: InitTargetArg,
        #[arg(default_value = ".")]
        root: PathBuf,
    },
    Watch {
        #[arg(default_value = ".")]
        root: PathBuf,
    },
    Changes {
        #[arg(default_value = ".")]
        root: PathBuf,
    },
    Hot {
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum EditOpArg {
    Replace,
    Insert,
    Delete,
}

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
pub enum InitTargetArg {
    All,
    Codex,
    Claude,
    Cursor,
}
