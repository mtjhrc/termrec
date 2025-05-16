use crate::file_format::{load_recording, parse_event_cmdline, RecordingEvent};
use crate::utils::find_subslice;
use anyhow::{bail, Context};
use clap::ArgGroup;
use clap::Parser;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

#[derive(Parser)]
#[command(group(
    ArgGroup::new("flags")
        .args(&["to_frame", "to_frame_with_text", "to_event"])
        .required(true)
))]

/// Measure time between events in a recording
pub struct MeasureCmd {
    #[clap(long, short = 'd')]
    recording_dir: PathBuf,

    // Only search for from_event and to_frame/to_event before this event
    #[clap(long)]
    before_event: Option<OsString>,

    // Only search for from_event and to_frame/to_event after this event
    #[clap(long)]
    after_event: Option<OsString>,

    // The event to measure time from
    #[clap(long)]
    from_event: OsString,

    #[clap(long)]
    delete_mosh_predict: bool,

    /// Path to a file containing a reference frame to measure up to
    #[clap(long)]
    to_frame: Option<PathBuf>,

    /// Search for a frame containing text
    #[clap(long)]
    to_frame_with_text: Option<OsString>,

    // The event to measure time until
    #[clap(long)]
    to_event: Option<OsString>,

    /// Print the timestamp in automatically selected human units, otherwise always uses microseconds
    #[clap(long, short = 'u')]
    human_units: bool,
}

impl MeasureCmd {
    pub fn run(self) -> anyhow::Result<()> {
        let recording = load_recording(&self.recording_dir.join("recording.termrec"))
            .context("Failed to load recording")?;

        let after_event = self
            .before_event
            .as_deref()
            .map(parse_event_cmdline)
            .transpose()
            .context("Invalid --after-event")?;

        let before_event = self
            .before_event
            .as_deref()
            .map(parse_event_cmdline)
            .transpose()
            .context("Invalid --before-event")?;

        let from_event = parse_event_cmdline(&self.from_event).context("Invalid --from-event")?;

        let recording = filter_only_after_and_before_events(recording, after_event, before_event);

        let delta = if let Some(to_event) = self.to_event {
            let to_event = parse_event_cmdline(&to_event).context("Invalid --to-event")?;

            let mut start = None;
            let mut end = None;

            for (timestamp, event) in recording {
                if event == from_event {
                    if let Some(start) = start {
                        log::warn!("Found multiple --from-event: {:?} and {:?}", start, event);
                    }
                    start = Some(timestamp);
                }
                if event == to_event {
                    end = Some(timestamp);
                    break;
                }
            }

            end.context("Didn't find --to_event")? - start.context("Didn't find --from_event")?
        } else
        /* to_frame/to_frame_with text */
        {
            let matches: Box<dyn Fn(&[u8]) -> bool> = if let Some(to_frame) = self.to_frame {
                let reference_frame =
                    fs::read(to_frame).context("Specified `to_frame` file does not exist.")?;

                Box::new(move |frame_contents| {
                    if self.delete_mosh_predict {
                        reference_frame == delete_mosh_predict(frame_contents)
                    } else {
                        reference_frame == frame_contents
                    }
                })
            } else if let Some(data) = self.to_frame_with_text {
                Box::new(move |frame_contents| {
                    if self.delete_mosh_predict {
                        find_subslice(&delete_mosh_predict(frame_contents), data.as_bytes())
                            .is_some()
                    } else {
                        find_subslice(frame_contents, data.as_bytes()).is_some()
                    }
                })
            } else {
                unreachable!()
            };

            measure(&matches, &from_event, &recording, &self.recording_dir)?
        };

        if self.human_units {
            println!("{delta:?}")
        } else {
            println!("{delta}", delta = delta.as_micros())
        }

        Ok(())
    }
}

pub fn measure(
    frame_matches: &impl Fn(&[u8]) -> bool,
    from_event: &RecordingEvent,
    recording: &[(Duration, RecordingEvent)],
    recording_dir: &Path,
) -> anyhow::Result<Duration> {
    let timestamp_from =
        find_event_time(from_event, recording).context("Didn't find --from-event")?;
    let timestamp_to = find_timestamp_of_frame(frame_matches, recording, recording_dir)
        .context("Didn't find --to-frame")?;

    if timestamp_to < timestamp_from {
        bail!(
            "Event happened at {timestamp_from:?}, but frame appeared sooner at {timestamp_to:?}."
        );
    }

    let delta = timestamp_to - timestamp_from;
    Ok(delta)
}

// FIXME: this seems broken?
fn delete_mosh_predict(data: &[u8]) -> Vec<u8> {
    let mut data = Vec::from(data);
    for escape in [b"\x1B[4m", b"\x1B[0m"] {
        while let Some(index) = find_subslice(&data, escape) {
            data.drain(index..index + escape.len());
        }
    }
    data
}

fn filter_only_after_and_before_events(
    events: Vec<(Duration, RecordingEvent)>,
    after_event: Option<RecordingEvent>,
    before_event: Option<RecordingEvent>,
) -> Vec<(Duration, RecordingEvent)> {
    if after_event.is_none() && before_event.is_none() {
        return events;
    }
    let mut result = Vec::new();
    let mut in_range = after_event.is_none();

    for (timestamp, event) in events {
        if after_event
            .as_ref()
            .is_some_and(|after_event| after_event == &event)
        {
            in_range = true;
            continue; // skip after_event itself
        }

        if before_event
            .as_ref()
            .is_some_and(|before_event| before_event == &event)
        {
            if in_range {
                break;
            } else {
                continue;
            }
        }

        if in_range {
            result.push((timestamp, event));
        }
    }

    result
}

fn find_event_time(
    reference_event: &RecordingEvent,
    recording: &[(Duration, RecordingEvent)],
) -> Option<Duration> {
    recording
        .iter()
        .find(|(_timestamp, recording_event)| reference_event == recording_event)
        .map(|(timestamp, _)| *timestamp)
}

fn find_timestamp_of_frame(
    frame_matches: &impl Fn(&[u8]) -> bool,
    recording: &[(Duration, RecordingEvent)],
    frames_dir: &Path,
) -> anyhow::Result<Duration> {
    for (timestamp, _event) in recording {
        let filename = format!("frame_{}", timestamp.as_micros());

        let file_contents = match fs::read(frames_dir.join(&filename)) {
            Ok(contents) => contents,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => Err(e).context("Failed to read frame: {filename}")?,
        };

        if frame_matches(&file_contents) {
            return Ok(*timestamp);
        }
    }

    bail!("Frame not found in directory");
}
