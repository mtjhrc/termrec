use crate::cmd::transform::TransformCmd;
use crate::file_format::{
    load_input, save_recording_termrec, InputEvent, RecordingEvent, SimulationEvent,
};
use crate::utils::find_subslice;
use anyhow::{bail, Context};
use clap::Parser;
use nix::errno::Errno;
use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::pty::{forkpty, ForkptyResult, Winsize};
use nix::sys::select::{select, FdSet};
use nix::sys::signal::{SigSet, Signal};
use nix::sys::signalfd::{SfdFlags, SignalFd};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{read, Pid};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::Receiver;
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};
use std::{fs, thread};

/// Run a program and record it's terminal IO
#[derive(Parser)]
pub struct RecordCmd {
    /// Input keystrokes to simulate
    #[arg(short, long)]
    pub input: Option<PathBuf>,

    #[arg(short, long)]
    pub verbose: bool,

    #[arg(long)]
    /// Redirect child stderr to a file/pipe/...
    pub child_stderr: Option<PathBuf>,

    /// Output file to save the recording to
    #[clap(
        short = 'o',
        long,
        conflicts_with = "output_dir",
        required_unless_present = "output_dir"
    )]
    pub output: Option<PathBuf>,

    /// Output directory to save the recording and it's individual frames
    #[clap(
        short = 'd',
        long,
        conflicts_with = "output",
        required_unless_present = "output"
    )]
    pub output_dir: Option<PathBuf>,

    pub command: Vec<String>,
}

impl RecordCmd {
    pub(crate) fn run(self) -> anyhow::Result<()> {
        if let Some(output) = self.output {
            record_cmd(
                &output,
                self.child_stderr.as_deref(),
                self.input.as_deref(),
                &self.command,
                self.verbose,
            )?;
        } else if let Some(output_dir) = self.output_dir {
            // Allow existing empty directory or create a new directory
            let output_is_empty_dir =
                fs::read_dir(&output_dir).is_ok_and(|mut d| d.next().is_none());
            if !output_is_empty_dir {
                fs::create_dir(&output_dir).context("Failed to create output directory")?;
            }

            let recording_path = output_dir.join("recording.termrec");
            record_cmd(
                &recording_path,
                self.child_stderr.as_deref(),
                self.input.as_deref(),
                &self.command,
                self.verbose,
            )?;
            TransformCmd {
                recording: recording_path,
                output_dir,
            }
            .run()?;
        } else {
            unreachable!();
        }

        Ok(())
    }
}

struct Recorder {
    start: SystemTime,
    read_buffer: Box<[u8]>,
    events: Vec<(Duration, RecordingEvent)>,
    data_tx: Option<mpsc::Sender<Msg>>,
}

impl Recorder {
    const READ_BUFFER_SIZE: usize = 2048 * 2048; // Same as mosh maximum terminal size

    fn begin(time_start: SystemTime, data_tx: Option<mpsc::Sender<Msg>>) -> Self {
        Self {
            start: time_start,
            read_buffer: Box::new([0; Self::READ_BUFFER_SIZE]),
            events: Vec::new(),
            data_tx,
        }
    }

    fn record(&mut self, data: Arc<[u8]>) {
        let timestamp = self.start.elapsed().unwrap();
        log::trace!("Out: {data:?}, {:?}", String::from_utf8_lossy(&data[..]));

        if let Some(data_tx) = &self.data_tx {
            // The input thread could quit early, ignore the error, and don't attempt to send again
            if data_tx.send(Msg::Data(data.clone())).is_err() {
                self.data_tx = None;
            }
        }
        self.events.push((timestamp, RecordingEvent::Output(data)));
    }

    fn record_from_fd(&mut self, fd: BorrowedFd) -> anyhow::Result<()> {
        loop {
            match read(fd.as_raw_fd(), &mut self.read_buffer) {
                Ok(0) | Err(Errno::EAGAIN) | Err(Errno::EIO) => break Ok(()),
                Ok(n) => self.record(Arc::from(&self.read_buffer[..n])),
                Err(e) => Err(e).context("read from term")?,
            }
        }
    }

    fn finish(self) -> Vec<(Duration, RecordingEvent)> {
        if let Some(tx) = self.data_tx {
            let _ = tx.send(Msg::End);
        }
        self.events
    }
}

fn make_nonblocking(fd: RawFd) -> nix::Result<()> {
    let flags = fcntl(fd, FcntlArg::F_GETFL)?;
    let new_flags = OFlag::from_bits_retain(flags) | OFlag::O_NONBLOCK;
    fcntl(fd, FcntlArg::F_SETFL(new_flags))?;
    Ok(())
}

fn record_term(term: OwnedFd, child: Pid, recorder: &mut Recorder) -> anyhow::Result<()> {
    make_nonblocking(term.as_raw_fd()).context("Make term fd nonblocking")?;

    let term_fd = term.as_fd();

    let mut rfds = FdSet::new();
    let mut sigmask = SigSet::empty();
    sigmask.add(Signal::SIGCHLD);
    sigmask.thread_block().unwrap();
    let sigchild =
        SignalFd::with_flags(&sigmask, SfdFlags::SFD_NONBLOCK).context("Create SignalFd")?;
    let sigchild_fd = sigchild.as_fd();

    loop {
        rfds.insert(term_fd);
        rfds.insert(sigchild_fd.as_fd());
        select(
            Some(sigchild_fd.as_raw_fd() + 1),
            &mut rfds,
            None,
            None,
            None,
        )
        .unwrap();

        if rfds.contains(term_fd) {
            recorder.record_from_fd(term_fd)?;
        }

        if let Ok(Some(_)) = sigchild.read_signal() {
            match waitpid(child, Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::StillAlive) => (),
                Ok(status) => {
                    log::trace!("Child process exited: {status:?}");
                    return Ok(());
                }
                Err(err) => bail!("WaitPid failed {err}"),
            }
        }
    }
}

enum Msg {
    Data(Arc<[u8]>),
    End,
}

fn block_until_found_needle(
    control_rx: &Receiver<Msg>,
    collected_data: &mut Vec<u8>,
    needle: &[u8],
    verbose: bool,
) -> bool {
    loop {
        match control_rx.recv().expect("Recording thread panicked") {
            Msg::Data(data) => {
                collected_data.extend(&data[..]);
                if let Some(index) = find_subslice(collected_data, needle) {
                    let drain = collected_data.drain(..index + needle.len());
                    if verbose {
                        let drained: Vec<u8> = drain.collect();
                        log::trace!("{needle:?}({l}bytes) IN {collected_data:?}({l2}bytes), draining {drained:?}", l = needle.len(), l2 = collected_data.len() );
                    }

                    return true;
                }
                if verbose {
                    log::trace!(
                        "{:?} NOT IN {:?}",
                        String::from_utf8_lossy(needle),
                        String::from_utf8_lossy(collected_data),
                    );
                }
            }
            Msg::End => {
                log::warn!("Quit before barrier unlocked");
                return false;
            }
        }
    }
}

fn spawn_input_thread(
    time_start: SystemTime,
    term_fd: OwnedFd,
    input_events: Vec<SimulationEvent>,
    control_rx: Receiver<Msg>,
    verbose: bool,
) -> JoinHandle<Vec<(Duration, RecordingEvent)>> {
    thread::spawn(move || {
        let mut recorded_events = Vec::with_capacity(input_events.len());
        let mut collected_data: Vec<u8> = Vec::with_capacity(Recorder::READ_BUFFER_SIZE);
        let mut out = File::from(term_fd);
        let mut last_timestamp = Duration::from_secs(0);

        for event in input_events {
            match event {
                SimulationEvent::Input(InputEvent { timestamp, data }) => {
                    let begin = SystemTime::now();
                    if timestamp >= last_timestamp {
                        let delta = timestamp - last_timestamp;
                        thread::sleep(delta);
                    } else {
                        log::warn!(
                            "WARNING: Input thread is behind: {:?}",
                            last_timestamp - timestamp
                        )
                    }
                    out.write_all(&data).unwrap();
                    log::trace!("Wrote input: {data:?}");
                    recorded_events.push((
                        time_start.elapsed().unwrap(),
                        RecordingEvent::InputRealized(data),
                    ));
                    last_timestamp += begin.elapsed().unwrap();
                }
                SimulationEvent::WaitBarrier(needle) => {
                    log::debug!("Wait: {needle:?}");

                    if !block_until_found_needle(&control_rx, &mut collected_data, &needle, verbose)
                    {
                        return recorded_events;
                    }
                    recorded_events.push((
                        time_start.elapsed().unwrap(),
                        RecordingEvent::BarrierUnlocked(needle),
                    ));
                    last_timestamp = Duration::from_secs(0);
                }
                SimulationEvent::Sleep(duration) => {
                    thread::sleep(duration);
                    recorded_events.push((
                        time_start.elapsed().unwrap(),
                        RecordingEvent::SleepFinished(duration),
                    ));
                    last_timestamp = Duration::from_secs(0);
                }
                SimulationEvent::Marker(data) => recorded_events
                    .push((time_start.elapsed().unwrap(), RecordingEvent::Marker(data))),
            }
        }
        recorded_events
    })
}

fn record_cmd(
    output: &Path,
    child_stderr: Option<&Path>,
    input: Option<&Path>,
    command: &[String],
    verbose: bool,
) -> anyhow::Result<()> {
    let terminal_size = Winsize {
        ws_row: 24,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };

    let input_events = if let Some(input) = input {
        load_input(input).context("Failed to load input")?
    } else {
        Vec::new()
    };

    let child_stderr = if let Some(child_stderr) = child_stderr {
        Some(
            OpenOptions::new()
                .write(true)
                .open(child_stderr)
                .context("Failed to open child stderr")?,
        )
    } else {
        None
    };

    let f = unsafe { forkpty(Some(&terminal_size), None) }.expect("Failed to fork pty");
    match f {
        ForkptyResult::Parent { child, master } => {
            drop(child_stderr);
            let time_start = SystemTime::now();

            let (tx, input_thread) = if !input_events.is_empty() {
                let (tx, rx) = mpsc::channel();
                let input_thread = spawn_input_thread(
                    time_start,
                    master.try_clone().unwrap(),
                    input_events,
                    rx,
                    verbose,
                );
                (Some(tx), Some(input_thread))
            } else {
                (None, None)
            };

            let mut recorder = Recorder::begin(time_start, tx);
            record_term(master, child, &mut recorder)?;

            let mut events = recorder.finish();
            if let Some(input_thread) = input_thread {
                match input_thread.join() {
                    Ok(input_thread_events) => {
                        events.extend(input_thread_events);
                        events.sort_by_key(|(timestamp, _event)| *timestamp)
                    }
                    Err(_) => {
                        bail!("FAILED to finish: input thread panicked");
                    }
                }
            }

            save_recording_termrec(events, output).context("Save recording")?;
        }
        ForkptyResult::Child => {
            let mut cmd = Command::new(&command[0]);
            cmd.args(&command[1..]);
            if let Some(child_stderr) = child_stderr {
                cmd.stderr(child_stderr);
            };

            let err = cmd.exec();
            bail!("Failed to exec: {err}");
        }
    }
    Ok(())
}
