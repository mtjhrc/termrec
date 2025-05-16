pub mod cmd;
pub mod event;
pub mod file_format;
pub mod unbuffered_stdout;
pub mod utils;

use crate::cmd::benchmark::BenchmarkCmd;
use crate::cmd::controlled_play::ControlledPlayCmd;
use crate::cmd::measure_cmd::MeasureCmd;
use crate::cmd::play::PlayCmd;
use crate::cmd::record::RecordCmd;
use crate::cmd::transform::TransformCmd;
use clap::{Parser, Subcommand};
use log::LevelFilter;

#[derive(Subcommand)]
enum CliCommand {
    Play(PlayCmd),
    ControlledPlay(ControlledPlayCmd),
    Transform(TransformCmd),
    Record(RecordCmd),
    Measure(MeasureCmd),
    Benchmark(BenchmarkCmd),
}

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: CliCommand,
}

fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    env_logger::builder()
        .filter_level(LevelFilter::Warn)
        .parse_default_env()
        .init();
    match args.command {
        CliCommand::Transform(cmd) => cmd.run(),
        CliCommand::ControlledPlay(cmd) => cmd.run(),
        CliCommand::Play(cmd) => cmd.run(),
        CliCommand::Record(cmd) => cmd.run(),
        CliCommand::Measure(cmd) => cmd.run(),
        CliCommand::Benchmark(cmd) => cmd.run(),
    }
}
