use std::io::{self, Read, Seek, SeekFrom, Write};
use std::time::{Duration, Instant};

/// A read-only partition window. Physical I/O stays sector-aligned and within
/// the partition even when a filesystem asks for individual unaligned bytes.
pub struct Partition<R> {
    source: R,
    base: u64,
    size: u64,
    position: u64,
    sector: u32,
    remaining: u64,
    operations: u32,
    deadline: Instant,
}

impl<R: Read + Seek> Partition<R> {
    pub fn new(source: R, base: u64, size: u64, sector: u32) -> io::Result<Self> {
        if !matches!(sector, 512 | 4096)
            || !base.is_multiple_of(u64::from(sector))
            || !size.is_multiple_of(u64::from(sector))
            || size == 0
            || base.checked_add(size).is_none()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid partition geometry",
            ));
        }
        Ok(Self {
            source,
            base,
            size,
            position: 0,
            sector,
            remaining: 1024 * 1024 * 1024,
            operations: 1_000_000,
            deadline: Instant::now() + Duration::from_secs(30),
        })
    }
}

impl<R: Read + Seek> Read for Partition<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() || self.position >= self.size {
            return Ok(0);
        }
        let sector = u64::from(self.sector);
        let start = self.position / sector * sector;
        let skip = (self.position - start) as usize;
        let count = out
            .len()
            .min(64 * 1024 - skip)
            .min((self.size - self.position) as usize);
        let length = (skip + count).next_multiple_of(self.sector as usize);
        if self.remaining < length as u64 || self.operations == 0 || Instant::now() >= self.deadline
        {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Raw filesystem inspection limit exceeded",
            ));
        }
        self.remaining -= length as u64;
        self.operations -= 1;
        let mut buffer = vec![0; length];
        self.source.seek(SeekFrom::Start(self.base + start))?;
        self.source.read_exact(&mut buffer)?;
        out[..count].copy_from_slice(&buffer[skip..skip + count]);
        self.position += count as u64;
        Ok(count)
    }
}

impl<R: Read + Seek> Seek for Partition<R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(n) => i128::from(self.position) + i128::from(n),
            SeekFrom::End(n) => i128::from(self.size) + i128::from(n),
        };
        if position < 0 || position > i128::from(u64::MAX) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Seek {position} outside partition of {} bytes", self.size),
            ));
        }
        self.position = position as u64;
        Ok(self.position)
    }
}

// fstool requires a Write interface even for reads. Never forward writes to
// the source, which deliberately only implements Read + Seek.
impl<R> Write for Partition<R> {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Raw filesystem access is read-only",
        ))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl<R: Read + Seek + Send> fstool::block::BlockDevice for Partition<R> {
    fn block_size(&self) -> u32 {
        self.sector
    }
    fn total_size(&self) -> u64 {
        self.size
    }
    fn sync(&mut self) -> fstool::Result<()> {
        Ok(())
    }
}

pub struct ExtSource<R>(pub Partition<R>);
impl<R: Read + Seek> ext4_view::Ext4Read for ExtSource<R> {
    fn read(
        &mut self,
        offset: u64,
        out: &mut [u8],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.0.seek(SeekFrom::Start(offset))?;
        self.0.read_exact(out)?;
        Ok(())
    }
}
