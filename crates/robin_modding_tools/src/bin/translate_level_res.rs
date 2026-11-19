//! Rewrite a `.res` text table from a Lithuanian translation table.
//!
//!   cargo run -p robin_modding_tools --bin translate_level_res -- \
//!     --input 2047/data/Text/Level.res --translations lt.toml --output pack/
//!
//! Only the listed strings change. Every other node of the archive, including
//! its pictures, is copied byte for byte, so the output is a shippable
//! `Level.res` for a partially translated language pack.
#![deny(clippy::print_stdout, clippy::print_stderr)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(clap::Parser)]
#[command(about = "Rewrite a Robin Hood .res text table from a translation table")]
struct Args {
    /// Source `.res` archive (typically the retail `Level.res`).
    #[arg(long)]
    input: PathBuf,
    /// TOML translation table with `[[line]]` entries of `id`, `index`, `text`.
    #[arg(long)]
    translations: PathBuf,
    /// Directory that receives the rewritten `Level.res`.
    #[arg(long)]
    output: PathBuf,
}

#[derive(Deserialize)]
struct TranslationFile {
    #[serde(default)]
    line: Vec<Line>,
}

#[derive(Deserialize)]
struct Line {
    id: i32,
    index: usize,
    text: String,
}

fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let args = <Args as clap::Parser>::parse();
    match run(&args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("error: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> Result<()> {
    let bytes =
        std::fs::read(&args.input).with_context(|| format!("read {}", args.input.display()))?;
    let source = std::fs::read_to_string(&args.translations)
        .with_context(|| format!("read {}", args.translations.display()))?;
    let table: TranslationFile = toml::from_str(&source)
        .with_context(|| format!("parse {}", args.translations.display()))?;
    if table.line.is_empty() {
        bail!(
            "{} contains no [[line]] entries",
            args.translations.display()
        );
    }

    let mut manager = robin_assets::resource_manager::ResourceManager::legacy_tool();
    manager.attach_resource_file(
        args.input
            .to_str()
            .context("input path is not valid UTF-8")?,
    )?;

    let mut touched: BTreeMap<i32, ()> = BTreeMap::new();
    for line in &table.line {
        manager
            .set_string(line.id, line.index, line.text.clone())
            .with_context(|| {
                format!(
                    "{}: id {} index {}",
                    args.translations.display(),
                    line.id,
                    line.index
                )
            })?;
        touched.insert(line.id, ());
    }

    let mut tables = BTreeMap::new();
    for id in touched.keys() {
        tables.insert(*id, manager.get_strings(*id)?.to_vec());
    }

    let spliced = robin_assets::resource_manager::splice_text_tables(&bytes, &tables)?;
    std::fs::create_dir_all(&args.output)
        .with_context(|| format!("create {}", args.output.display()))?;
    let output = args.output.join("Level.res");
    std::fs::write(&output, &spliced).with_context(|| format!("write {}", output.display()))?;
    Ok(())
}
