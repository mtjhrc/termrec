use crate::event::EventFile;
use crate::file_format::{filter_output_events, load_recording};
use anyhow::{bail, Context};
use clap::Parser;
use std::env;
use std::fs::File;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Transform a termrec recoding into idividual frames
#[derive(Parser)]
pub struct TransformCmd {
    #[arg(short, long)]
    pub output_dir: PathBuf,

    pub recording: PathBuf,
}

impl TransformCmd {
    pub fn run(self) -> anyhow::Result<()> {
        if !self.output_dir.is_dir() {
            bail!("Output is not a directory");
        }
        let mut write_event = EventFile::create(self.output_dir.join(".termrec-write-event"))?;
        let mut finished_event =
            EventFile::create(self.output_dir.join(".termrec-finished-event"))?;

        let recording = load_recording(&self.recording).context("Failed to load recording")?;
        let events = filter_output_events(recording);

        let current_exe = env::current_exe().context("Failed to get current executable path")?;
        let tmux_session_name = "transform-rec-help";

        let create_session_output = Command::new("tmux")
            .arg("new-session")
            .arg("-P")
            .arg("-s")
            .arg(tmux_session_name)
            .arg("-d")
            .arg("--")
            .arg(current_exe)
            .arg("controlled-play")
            .arg("--write-event")
            .arg(write_event.path())
            .arg("--finished-event")
            .arg(finished_event.path())
            .arg(&self.recording)
            .output()
            .context("Failed to execute tmux")?;

        if create_session_output.status.code().is_none_or(|s| s != 0) {
            bail!("Failed to create tmux session: {create_session_output:?}");
        }

        for (timestamp, _data) in events.iter() {
            write_event.signal()?;
            finished_event.wait()?;

            let frame_timestamp: u64 = timestamp
                .as_micros()
                .try_into()
                .expect("Timestamp too large");

            let output_frame_path = &self.output_dir.join(format!("frame_{}", frame_timestamp));
            let output_frame_file =
                File::create(output_frame_path).context("Failed to create output file")?;
            let out = Command::new("tmux")
                .arg("capture-pane")
                .arg("-p")
                .arg("-e") // escape sequences
                .arg("-J")
                .arg("-t")
                .arg(tmux_session_name)
                .stdout(Stdio::from(output_frame_file))
                .output()
                .context("Failed to execute tmux")?;

            if out.status.code().is_none_or(|s| s != 0) {
                bail!("Failed to capture frame using tmux: {out:?}");
            }
        }
        write_event.signal()?;
        Ok(())
    }
}
