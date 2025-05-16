use anyhow::bail;
use clap::Parser;
use std::ffi::OsString;
use std::path::PathBuf;

const DEFAULT_RECORDING_DIR: &str = "/tmp/termrec-benchmark";

/// Run the specified command multiple times and measure the event
#[derive(Parser)]
pub struct BenchmarkCmd {
    /// Input keystrokes to simulate
    #[arg(long, short)]
    input: Option<PathBuf>,

    #[arg(long, short = 'n')]
    samples: u32,

    #[clap(long, short = 'd', default_value=DEFAULT_RECORDING_DIR)]
    recording_dir: PathBuf,

    #[clap(long, short = 'f')]
    from_event: OsString,

    /// Path to a file containing a reference frame to measure up to
    #[clap(long, short = 't')]
    to_frame: PathBuf,

    /// Print the timestamp in automatically selected human units, otherwise always uses microseconds
    #[clap(long, short = 'u')]
    human_units: bool,

    // The command and arguments to benchmark
    #[clap(required = true)]
    command: Vec<String>,
}

impl BenchmarkCmd {
    pub fn run(self) -> anyhow::Result<()> {
        bail!("Not implemented");
        /*
               if self.recording_dir.exists() {
                   bail!("Tmp directory ({DEFAULT_RECORDING_DIR}) exists, consider removing it or use a different dir. ")
               }

               let event_from = parse_event_cmdline(&self.from_event)?;
               let frame_to =
                   fs::read(self.to_frame).context("Failed to read reference frame (--frame_to)")?;

               for _ in 0..self.samples {
                   RecordCmd {
                       input: self.input.clone(),
                       output: None,
                       output_dir: Some(self.recording_dir.clone()),
                       command: self.command.clone(),
                   }
                   .run()?;

                   let delta = measure(&frame_to[..], &event_from, &self.recording_dir)?;
                   println!("{delta:?}");

                   fs::remove_dir_all(&self.recording_dir)
                       .context("Failed to delete recording tmp directory")?
               }

               Ok(())
        */
    }
}
