//! A bounded reader of the tar the engine gives back for a container's
//! folder (`download_from_container`). Only the entries the collector is
//! expected to write are taken, each to where the caller says; anything
//! else (links, devices, other names, oversized entries) fails the read, so
//! a result archive can never write outside what was asked for.

use std::io;
use std::path::PathBuf;

use tokio::io::AsyncWriteExt;

/// Where one expected entry goes.
pub enum Sink {
    /// Kept in memory, up to the limit.
    Memory(Vec<u8>),
    /// Written to a file, up to the limit.
    File(PathBuf, Option<tokio::fs::File>),
}

/// One entry the archive may hold.
pub struct Expected {
    pub name: &'static str,
    pub max_bytes: u64,
    pub sink: Sink,
    pub found: bool,
}

enum State {
    Header,
    Body {
        index: Option<usize>,
        left: u64,
        pad: u64,
    },
    Done,
}

/// Feeds the archive's bytes, a chunk at a time, into the expected entries.
pub struct TarReader {
    expected: Vec<Expected>,
    buffer: Vec<u8>,
    state: State,
}

impl TarReader {
    pub fn new(expected: Vec<Expected>) -> Self {
        Self {
            expected,
            buffer: Vec::new(),
            state: State::Header,
        }
    }

    pub async fn feed(&mut self, chunk: &[u8]) -> io::Result<()> {
        self.buffer.extend_from_slice(chunk);
        loop {
            match &mut self.state {
                State::Done => {
                    self.buffer.clear();
                    return Ok(());
                }
                State::Header => {
                    if self.buffer.len() < 512 {
                        return Ok(());
                    }
                    let header: Vec<u8> = self.buffer.drain(..512).collect();
                    if header.iter().all(|b| *b == 0) {
                        self.state = State::Done;
                        continue;
                    }
                    let (name, size, kind) = parse_header(&header)?;
                    let index = match kind {
                        // A folder entry carries nothing.
                        b'5' => None,
                        b'0' | 0 => {
                            let index = self
                                .expected
                                .iter()
                                .position(|e| e.name == name)
                                .ok_or_else(|| {
                                    io::Error::other(format!("unexpected archive entry {name}"))
                                })?;
                            let entry = &mut self.expected[index];
                            if entry.found {
                                return Err(io::Error::other(format!(
                                    "archive entry {name} appears twice"
                                )));
                            }
                            if size > entry.max_bytes {
                                return Err(io::Error::other(format!(
                                    "archive entry {name} is over its limit"
                                )));
                            }
                            entry.found = true;
                            if let Sink::File(path, file) = &mut entry.sink {
                                *file = Some(tokio::fs::File::create(&path).await?);
                            }
                            Some(index)
                        }
                        other => {
                            return Err(io::Error::other(format!(
                                "archive entry {name} is not a regular file (type {other})"
                            )))
                        }
                    };
                    self.state = State::Body {
                        index,
                        left: size,
                        pad: (512 - size % 512) % 512,
                    };
                }
                State::Body { index, left, pad } => {
                    if *left > 0 {
                        if self.buffer.is_empty() {
                            return Ok(());
                        }
                        let take = (*left).min(self.buffer.len() as u64) as usize;
                        let piece: Vec<u8> = self.buffer.drain(..take).collect();
                        *left -= take as u64;
                        if let Some(index) = *index {
                            match &mut self.expected[index].sink {
                                Sink::Memory(bytes) => bytes.extend_from_slice(&piece),
                                Sink::File(_, Some(file)) => file.write_all(&piece).await?,
                                Sink::File(..) => unreachable!("opened with the header"),
                            }
                        }
                        continue;
                    }
                    if *pad > 0 {
                        if self.buffer.is_empty() {
                            return Ok(());
                        }
                        let take = (*pad).min(self.buffer.len() as u64) as usize;
                        self.buffer.drain(..take);
                        *pad -= take as u64;
                        continue;
                    }
                    if let Some(index) = *index {
                        if let Sink::File(_, Some(file)) = &mut self.expected[index].sink {
                            file.flush().await?;
                            file.sync_all().await?;
                        }
                    }
                    self.state = State::Header;
                }
            }
        }
    }

    /// The entries, once the archive ended. An entry whose bytes were cut
    /// short (the stream ended mid-body) is an error.
    pub fn finish(self) -> io::Result<Vec<Expected>> {
        match self.state {
            State::Done | State::Header => Ok(self.expected),
            State::Body { .. } => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the archive ended inside an entry",
            )),
        }
    }
}

/// The name, size, and type of a ustar header; a name with a prefix is joined.
fn parse_header(h: &[u8]) -> io::Result<(String, u64, u8)> {
    let field = |from: usize, len: usize| -> &[u8] {
        let raw = &h[from..from + len];
        let end = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
        &raw[..end]
    };
    let text = |from: usize, len: usize| -> io::Result<String> {
        String::from_utf8(field(from, len).to_vec())
            .map_err(|_| io::Error::other("an archive entry's name is not UTF-8"))
    };
    let mut name = text(0, 100)?;
    let prefix = text(345, 155)?;
    if !prefix.is_empty() {
        name = format!("{prefix}/{name}");
    }
    let size_text = text(124, 12)?;
    let size = u64::from_str_radix(size_text.trim().trim_end_matches('\0'), 8)
        .map_err(|_| io::Error::other("an archive entry's size is not octal"))?;
    let kind = h[156];
    // A PAX extension header (type x or g) would carry attributes this reader
    // does not apply; the collector's entries need none.
    Ok((name, size, kind))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::image::header;

    fn entry(name: &str, bytes: &[u8]) -> Vec<u8> {
        let mut out = header(name, bytes.len()).to_vec();
        out.extend_from_slice(bytes);
        out.resize(out.len() + (512 - bytes.len() % 512) % 512, 0);
        out
    }

    #[tokio::test]
    async fn expected_entries_are_taken_and_others_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut tar = entry("out/manifest.json", b"{}");
        tar.extend(entry("out/result.bundle", &[7u8; 1000]));
        tar.extend(vec![0u8; 1024]);
        let path = dir.path().join("result.bundle");
        let mut reader = TarReader::new(vec![
            Expected {
                name: "out/manifest.json",
                max_bytes: 1024,
                sink: Sink::Memory(Vec::new()),
                found: false,
            },
            Expected {
                name: "out/result.bundle",
                max_bytes: 4096,
                sink: Sink::File(path.clone(), None),
                found: false,
            },
        ]);
        for chunk in tar.chunks(300) {
            reader.feed(chunk).await.unwrap();
        }
        let entries = reader.finish().unwrap();
        assert!(matches!(&entries[0].sink, Sink::Memory(b) if b == b"{}"));
        assert_eq!(std::fs::read(&path).unwrap(), vec![7u8; 1000]);

        let mut reader = TarReader::new(vec![Expected {
            name: "out/manifest.json",
            max_bytes: 1,
            sink: Sink::Memory(Vec::new()),
            found: false,
        }]);
        assert!(reader
            .feed(&entry("out/manifest.json", b"{}"))
            .await
            .is_err());
        let mut reader = TarReader::new(vec![]);
        assert!(reader.feed(&entry("etc/passwd", b"x")).await.is_err());
        let mut link = header("out/link", 0);
        link[156] = b'2';
        let mut reader = TarReader::new(vec![]);
        assert!(reader.feed(&link).await.is_err());
    }
}
