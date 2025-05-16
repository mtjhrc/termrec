use anyhow::{bail, Context};
use nix::errno::Errno;
use nix::sys::stat::Mode;
use nix::unistd::{mkfifo, unlink};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Utility to signal events over a named pipe
pub struct EventFile {
    pipe: File,
    path: PathBuf,
}

impl EventFile {
    pub fn create(path: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let path = path.into();
        match mkfifo(&path, Mode::S_IRUSR | Mode::S_IWUSR) {
            Ok(()) => {}
            Err(Errno::EEXIST) => {
                bail!("{path:?} already exists, maybe it is a leftover from a crashed termrec run?")
            }
            Err(e) => Err(e).context("mkfifo")?,
        }
        let pipe = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .context("Failed to open pipe used for EventFile")?;

        Ok(Self { pipe, path })
    }

    pub fn connect(path: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let path = path.into();
        let fifo = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .context("Failed to open pipe used for EventFile")?;
        unlink(&path).context("Failed to unlink the pipe")?;
        Ok(Self { pipe: fifo, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn signal(&mut self) -> anyhow::Result<()> {
        self.pipe.write_all(b".").context("EventFile::signal")?;
        Ok(())
    }

    pub fn wait(&mut self) -> anyhow::Result<()> {
        let mut buf = [0u8];
        while self.pipe.read(&mut buf).context("EventFile::wait")? == 0 {}
        Ok(())
    }
}
