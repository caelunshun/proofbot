use crate::config::Config;
use clap::Parser;

mod checker;
mod config;
mod constrained_file_editor;
mod context;
mod driver;
mod hacky_lean_parsing;
mod pretty_file_tree;
mod templates;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args = driver::Args::parse();

    driver::run_driver(&Config::load()?, &args)
}
