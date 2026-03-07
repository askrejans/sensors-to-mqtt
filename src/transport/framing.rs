//! Length-prefixed TCP framing utilities.
//!
//! The bridge sends responses as `[len_hi][len_lo][payload...]` where the
//! 2-byte big-endian header gives the byte count of the following payload.

use anyhow::{Context, Result};
use std::io::{self, Read};
use std::net::TcpStream;

/// Read one length-prefixed frame from `stream` into `buf`.
///
/// Returns an error if the frame length doesn't match `buf.len()`.
pub fn tcp_read_framed(stream: &mut TcpStream, buf: &mut [u8]) -> Result<()> {
    let mut len_buf = [0u8; 2];
    stream
        .read_exact(&mut len_buf)
        .context("TCP bridge: read frame length")?;
    let n = u16::from_be_bytes(len_buf) as usize;
    if n != buf.len() {
        anyhow::bail!(
            "TCP bridge: frame length mismatch — expected {} bytes, got {}",
            buf.len(),
            n
        );
    }
    stream
        .read_exact(buf)
        .context("TCP bridge: read frame payload")
}

/// Wraps a `TcpStream` and transparently strips length-prefixed frames,
/// presenting the payload bytes as a plain `Read` stream.
///
/// Each frame is read on demand when the internal buffer is exhausted.
/// Partial reads are served from the buffer without fetching a new frame.
pub struct FramedTcpReader {
    stream: TcpStream,
    buf: Vec<u8>,
    pos: usize,
}

impl FramedTcpReader {
    pub fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            buf: Vec::new(),
            pos: 0,
        }
    }
}

impl Read for FramedTcpReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.buf.len() {
            let mut len_buf = [0u8; 2];
            self.stream.read_exact(&mut len_buf)?;
            let n = u16::from_be_bytes(len_buf) as usize;
            self.buf.resize(n, 0);
            self.stream.read_exact(&mut self.buf)?;
            self.pos = 0;
        }
        let available = &self.buf[self.pos..];
        let to_copy = out.len().min(available.len());
        out[..to_copy].copy_from_slice(&available[..to_copy]);
        self.pos += to_copy;
        Ok(to_copy)
    }
}
