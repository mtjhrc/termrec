use anyhow::{anyhow, bail, ensure, Context};
use std::ffi::OsStr;
use std::fs::File;
use std::io::{BufRead, BufReader, ErrorKind, Read, Seek, SeekFrom, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

pub type Data = Arc<[u8]>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecordingEvent {
    Output(Data),
    InputRealized(Data),
    BarrierUnlocked(Data),
    SleepFinished(Duration),
    Marker(Data),
}

pub enum SimulationEvent {
    Input(InputEvent),
    WaitBarrier(Data),
    Sleep(Duration),
    Marker(Data),
}

pub struct InputEvent {
    pub timestamp: Duration,
    pub data: Data,
}

const TERMREC_RECORDING_HEADER: &[u8] = b"termrec:v1:rec:";
const TERMREC_INPUT_HEADER: &[u8] = b"termrec:v1:inp:";

pub fn parse_event_cmdline(arg: &OsStr) -> anyhow::Result<RecordingEvent> {
    let arg = arg.as_bytes();

    let event = arg.get(..2).ok_or(anyhow!("Event name missing"))?;
    let data = arg.get(2..).ok_or(anyhow!("Event data missing"))?;
    let data = Arc::from(data);
    let event = match event {
        b"o:" => RecordingEvent::Output(data),
        b"w:" => RecordingEvent::BarrierUnlocked(data),
        b"i:" => RecordingEvent::InputRealized(data),
        b"m:" => RecordingEvent::Marker(data),
        _ => bail!("Unknown/unsupported event: {event:?}"),
    };

    Ok(event)
}

/// Attempts to load a termrec or asciinema recording by autodetecting the format
pub fn load_recording(recording_file: &Path) -> anyhow::Result<Vec<(Duration, RecordingEvent)>> {
    let mut file = BufReader::new(File::open(recording_file).unwrap());

    let mut header_buf = [0u8; TERMREC_RECORDING_HEADER.len()];
    file.read_exact(&mut header_buf).expect("File too small");
    if header_buf == TERMREC_RECORDING_HEADER {
        load_recording_termec_format(file).context("Failed to load recording in termrec format")
    } else if header_buf == TERMREC_INPUT_HEADER {
        bail!("Invalid file: File is a termrec file, but not a recording. It is an input simulation file!");
    } else {
        file.seek(SeekFrom::Start(0))
            .context("Failed to seek input file, this is required to load asciinema format")?;
        load_recording_asciinema_format(file)
            .context("Failed to load recording in asciinema format")
    }
}

pub fn filter_output_events(input: Vec<(Duration, RecordingEvent)>) -> Vec<(Duration, Data)> {
    input
        .into_iter()
        .filter_map(|(timestamp, event)| match event {
            RecordingEvent::Output(data) => Some((timestamp, data)),
            _ => None,
        })
        .collect()
}

fn validitate_simulation_events(events: &[SimulationEvent]) -> anyhow::Result<()> {
    let mut last_timestamp = Duration::from_secs(0);
    for event in events {
        match event {
            SimulationEvent::Input(InputEvent { timestamp, data }) => {
                ensure!(*timestamp >= last_timestamp, "Invalid timestamp for event: {data:?}. Expected {timestamp:?} >= {last_timestamp:?}");
                last_timestamp = *timestamp;
            }
            SimulationEvent::WaitBarrier(_) => {
                last_timestamp = Duration::from_secs(0);
            }
            SimulationEvent::Sleep(_) => {
                last_timestamp = Duration::from_secs(0);
            }
            SimulationEvent::Marker(_) => (),
        }
    }
    Ok(())
}

pub fn load_input(file: &Path) -> anyhow::Result<Vec<SimulationEvent>> {
    let mut file = BufReader::new(File::open(file).context("Failed to open input file")?);
    let mut header_buf = [0u8; TERMREC_INPUT_HEADER.len()];
    file.read_exact(&mut header_buf)
        .context("Invalid file: unknown format")?;

    if header_buf == TERMREC_RECORDING_HEADER {
        bail!("Invalid file: File is a termrec file, but not an input file. It is a recording!");
    } else if header_buf != TERMREC_INPUT_HEADER {
        bail!("Invalid file: unknown format");
    }

    let mut events = Vec::new();
    let mut line_num = 0;
    loop {
        let mut cmd = [0u8; 2];

        match file.read_exact(&mut cmd) {
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => break,
            Err(e) => bail!("File read error: {e}"),
            Ok(()) => (),
        }

        let err_context = || format!("On line {line_num}");
        let event = match &cmd {
            b"i:" => {
                let timestamp_us = read_num(&mut file).with_context(err_context)?;
                let data = read_data(&mut file).with_context(err_context)?;

                SimulationEvent::Input(InputEvent {
                    timestamp: Duration::from_micros(timestamp_us),
                    data,
                })
            }
            b"w:" => {
                let data = read_data(&mut file).with_context(err_context)?;
                SimulationEvent::WaitBarrier(data)
            }
            b"s:" => {
                let duration = read_duration(&mut file).with_context(err_context)?;
                SimulationEvent::Sleep(duration)
            }
            b"m:" => {
                let data = read_data(&mut file).with_context(err_context)?;
                SimulationEvent::Marker(data)
            }
            b"--" => {
                read_line_comment(&mut file);
                continue;
            }
            // Ignore newlines (to make it easier to write the file by hand)
            b"\\\n" => {
                line_num += 1;
                continue;
            }
            b"\n\n" => {
                line_num += 2;
                continue;
            }
            other => {
                bail!(
                    "Unknown input command {other:?} ({:?}) (line {line_num})",
                    String::from_utf8_lossy(other)
                )
            }
        };
        events.push(event);
    }

    validitate_simulation_events(&events)?;
    Ok(events)
}

pub fn save_recording_termrec(
    events: Vec<(Duration, RecordingEvent)>,
    path: &Path,
) -> anyhow::Result<()> {
    let mut f = File::create(path).context("Failed to open output file")?;
    f.write_all(TERMREC_RECORDING_HEADER)?;
    f.write_all(b"\\\n")?;
    for (timestamp, event) in events {
        let timestamp: u64 = timestamp
            .as_micros()
            .try_into()
            .context("Timestamp too large")?;

        let write_cmd_data = |f: &mut File, cmd, data: Data| {
            let data_len: u64 = data.len() as u64;
            write!(f, "{cmd}:{timestamp}:{data_len}:")?;
            f.write_all(&data)?;
            write!(f, "\\\n")?;
            Ok::<_, anyhow::Error>(())
        };

        let write_cmd_duration = |f: &mut File, cmd, duration: Duration| {
            write!(f, "{cmd}:{timestamp}:{}:\\\n", duration.as_micros() as u64)?;
            Ok::<_, anyhow::Error>(())
        };

        match event {
            RecordingEvent::Marker(data) => write_cmd_data(&mut f, 'm', data),
            RecordingEvent::Output(data) => write_cmd_data(&mut f, 'o', data),
            RecordingEvent::InputRealized(data) => write_cmd_data(&mut f, 'i', data),
            RecordingEvent::SleepFinished(duration) => write_cmd_duration(&mut f, 's', duration),
            RecordingEvent::BarrierUnlocked(data) => write_cmd_data(&mut f, 'w', data),
        }.context("Failed to write to output file")?;
    }
    Ok(())
}

fn read_num(reader: &mut impl BufRead) -> anyhow::Result<u64> {
    let mut buf = Vec::new();
    let num_bytes = reader
        .read_until(b':', &mut buf)
        .context("Read num until separator")?;
    if num_bytes == 0 {
        bail!("Unexpected EOF");
    } else if num_bytes <= 1 || buf[num_bytes - 1] != b':' {
        bail!(
            "Expected ':' separator, {:?}",
            String::from_utf8_lossy(&buf)
        );
    }
    let num_str = std::str::from_utf8(&buf[..num_bytes - 1])
        .context("Expected UTF-8 representing a number")?;
    let num = u64::from_str(num_str).context("Expected a number")?;
    Ok(num)
}

fn read_line_comment(reader: &mut impl BufRead) {
    let mut buf = Vec::new();
    let _ = reader.read_until(b'\n', &mut buf);
}

fn read_duration(reader: &mut impl BufRead) -> anyhow::Result<Duration> {
    let num = read_num(reader)?;
    Ok(Duration::from_micros(num))
}

fn read_data(reader: &mut impl BufRead) -> anyhow::Result<Data> {
    let buf_len = read_num(reader)?;
    let mut data = vec![0u8; buf_len as usize];
    reader
        .read_exact(&mut data)
        .context("Partial file, expected more bytes")?;
    Ok(data.into())
}

fn load_recording_termec_format(
    mut file: BufReader<File>,
) -> anyhow::Result<Vec<(Duration, RecordingEvent)>> {
    let mut events = Vec::new();
    let mut line_num = 0;
    loop {
        let mut cmd = [0u8; 2];
        match file.read_exact(&mut cmd) {
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => break,
            Err(e) => bail!("File read error: {e}"),
            Ok(()) => (),
        }
        let err_context = || format!("On line {line_num}");
        let (timestamp, event) = match &cmd {
            b"o:" => {
                let timestamp = read_duration(&mut file).with_context(err_context)?;
                let data = read_data(&mut file).with_context(err_context)?;
                (timestamp, RecordingEvent::Output(data))
            }
            b"i:" => {
                let timestamp = read_duration(&mut file).with_context(err_context)?;
                let data = read_data(&mut file).with_context(err_context)?;
                (timestamp, RecordingEvent::InputRealized(data))
            }
            b"w:" => {
                let timestamp = read_duration(&mut file).with_context(err_context)?;
                let data = read_data(&mut file).with_context(err_context)?;
                (timestamp, RecordingEvent::BarrierUnlocked(data))
            }
            b"s:" => {
                let timestamp = read_duration(&mut file).with_context(err_context)?;
                let duration = read_duration(&mut file).with_context(err_context)?;
                (timestamp, RecordingEvent::SleepFinished(duration))
            }
            b"m:" => {
                let timestamp = read_duration(&mut file).with_context(err_context)?;
                let data = read_data(&mut file).with_context(err_context)?;
                (timestamp, RecordingEvent::Marker(data))
            }
            b"--" => {
                read_line_comment(&mut file);
                continue;
            }
            b"\\\n" => {
                line_num += 1;
                continue;
            }
            b"\n\n" => {
                line_num += 2;
                continue;
            }
            other => bail!("Unknown recording command {other:?}, line {line_num}"),
        };

        events.push((timestamp, event));
    }

    Ok(events)
}

fn load_recording_asciinema_format(
    file: BufReader<File>,
) -> anyhow::Result<Vec<(Duration, RecordingEvent)>> {
    file.lines()
        .skip(1)
        .flat_map(|line| {
            line.map_err(Into::into)
                .and_then(asciinema_line_to_event)
                .transpose()
        })
        .collect()
}

fn asciinema_line_to_event(line: String) -> anyhow::Result<Option<(Duration, RecordingEvent)>> {
    let parsed_json: serde_json::Value =
        serde_json::from_str(&line).context("Failed to parse json")?;
    let arr = parsed_json.as_array().context("Expected json array")?;

    let timestamp = arr[0].as_f64().context("Expected number")?;
    let event = arr[1].as_str().context("Expected string")?;
    match event {
        "m" => return Ok(None), // Marker - ignored/unsupported
        "o" => (),              // Output
        "r" => bail!("Resize unsuported"),
        _ => bail!("Unknown event: {event:?}"),
    }
    let data = arr[2].as_str().context("Expected string")?.to_string();

    let event = RecordingEvent::Output(Arc::from(data.as_bytes()));
    Ok(Some((Duration::from_secs_f64(timestamp), event)))
}
