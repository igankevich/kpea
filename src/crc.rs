use std::io::Error;
use std::io::Write;

/// Computes sum of all bytes.
pub struct CrcWriter<W: Write> {
    writer: W,
    sum: usize,
}

impl<W: Write> CrcWriter<W> {
    pub fn new(writer: W) -> Self {
        Self { writer, sum: 0 }
    }

    pub fn into_inner(self) -> W {
        self.writer
    }

    pub fn sum(&self) -> u32 {
        (self.sum & (u32::MAX as usize)) as u32
    }
}

impl<W: Write> Write for CrcWriter<W> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Error> {
        let n = self.writer.write(buf)?;
        for x in &buf[..n] {
            self.sum = self.sum.wrapping_add(*x as usize);
        }
        Ok(n)
    }

    fn flush(&mut self) -> Result<(), Error> {
        self.writer.flush()
    }
}
