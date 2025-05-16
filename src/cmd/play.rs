use crate::file_format::{filter_output_events, load_recording, Data};
use crate::unbuffered_stdout::UnbufferedStdout;
use anyhow::{bail, Context};
use clap::Parser;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, SystemTime};

/// Replay a saved termrec recording
#[derive(Parser)]
pub struct PlayCmd {
    #[clap(short, long, default_value_t = 1000)] //1ms
    max_accuracy_delta_us: u64,
    recording: PathBuf,
}

impl PlayCmd {
    pub fn run(self) -> anyhow::Result<()> {
        let recording = load_recording(&self.recording).context("Failed to load recording")?;
        let events: Vec<(Duration, Data)> = filter_output_events(recording);
        let max_delta = Duration::from_micros(self.max_accuracy_delta_us);

        let mut stdout = UnbufferedStdout::lock();
        let mut last_timestamp = Duration::from_secs(0);

        for (timestamp, data) in events {
            let begin = SystemTime::now();
            if timestamp >= last_timestamp {
                let delta = timestamp - last_timestamp;
                thread::sleep(delta);
            } else {
                let delta = last_timestamp - timestamp;
                if delta > max_delta {
                    bail!(
                        "Playback too slow: maximum delta {max_delta:?}, actual delta: {delta:?}"
                    );
                }
            }
            stdout.write_all(&data).context("Write to stdout")?;
            let elapsed = begin.elapsed()?;
            last_timestamp += elapsed;
        }
        Ok(())
    }
}
