use crate::event::EventFile;
use crate::file_format::{filter_output_events, load_recording};
use crate::unbuffered_stdout::UnbufferedStdout;
use anyhow::Context;
use clap::Parser;
use std::path::PathBuf;

/// Play the recording frame-by-frame (to be processed programatically)
#[derive(Parser)]
pub struct ControlledPlayCmd {
    recording: PathBuf,

    #[arg(long)]
    write_event: PathBuf,

    #[arg(long)]
    finished_event: PathBuf,
}

impl ControlledPlayCmd {
    pub fn run(self) -> anyhow::Result<()> {
        let recording = load_recording(&self.recording).context("Failed to load recording")?;
        let events = filter_output_events(recording);

        let mut write_event = EventFile::connect(self.write_event)?;
        let mut finished_event = EventFile::connect(self.finished_event)?;

        let mut stdout = UnbufferedStdout::lock();
        for (_, data) in events {
            write_event.wait()?;
            stdout
                .write_all(&data)
                .context("Failed to write to stdout")?;
            finished_event.signal()?;
        }
        write_event.wait()?;
        Ok(())
    }
}
