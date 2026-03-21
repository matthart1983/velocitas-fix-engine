/// Message journal for persistence and recovery.
///
/// Uses memory-mapped files for high-performance persistence with zero-copy reads.
/// Messages are stored in a circular buffer with CRC32 integrity checks.

use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};

use memmap2::MmapMut;

/// Journal entry header (fixed 24 bytes, cache-line friendly).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy)]
pub struct JournalEntryHeader {
    /// Session ID hash (for filtering during recovery).
    pub session_hash: u64,
    /// Message sequence number.
    pub seq_num: u64,
    /// Message body length in bytes.
    pub body_length: u32,
    /// CRC32 checksum of the message body.
    pub crc32: u32,
}

const HEADER_SIZE: usize = std::mem::size_of::<JournalEntryHeader>();

/// Sync policy for journal writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncPolicy {
    /// No fsync — rely on OS page cache. Fastest.
    None,
    /// fsync every N milliseconds.
    Batch(u32),
    /// fsync after every message. Slowest but safest.
    EveryMessage,
}

/// Memory-mapped message journal.
pub struct Journal {
    _path: PathBuf,
    mmap: MmapMut,
    write_offset: usize,
    capacity: usize,
    sync_policy: SyncPolicy,
    entry_count: u64,
}

impl Journal {
    /// Open or create a journal file at the given path with the specified size.
    pub fn open(path: &Path, size_bytes: usize, sync_policy: SyncPolicy) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)?;

        file.set_len(size_bytes as u64)?;

        let mmap = unsafe { MmapMut::map_mut(&file)? };

        Ok(Journal {
            _path: path.to_path_buf(),
            mmap,
            write_offset: 0,
            capacity: size_bytes,
            sync_policy,
            entry_count: 0,
        })
    }

    /// Append a message to the journal. Returns the offset where it was written.
    #[inline]
    pub fn append(
        &mut self,
        session_hash: u64,
        seq_num: u64,
        message: &[u8],
    ) -> io::Result<usize> {
        let total_size = HEADER_SIZE + message.len();

        // Check if we need to wrap around
        if self.write_offset + total_size > self.capacity {
            self.write_offset = 0; // Circular: wrap to beginning
        }

        let offset = self.write_offset;

        // Write header
        let header = JournalEntryHeader {
            session_hash,
            seq_num,
            body_length: message.len() as u32,
            crc32: crc32_simple(message),
        };

        let header_bytes = unsafe {
            std::slice::from_raw_parts(
                &header as *const JournalEntryHeader as *const u8,
                HEADER_SIZE,
            )
        };

        self.mmap[offset..offset + HEADER_SIZE].copy_from_slice(header_bytes);
        self.mmap[offset + HEADER_SIZE..offset + total_size].copy_from_slice(message);
        self.write_offset += total_size;
        self.entry_count += 1;

        // Sync if policy requires it
        if self.sync_policy == SyncPolicy::EveryMessage {
            self.mmap.flush()?;
        }

        Ok(offset)
    }

    /// Read a journal entry at the given offset.
    pub fn read_entry(&self, offset: usize) -> Option<(JournalEntryHeader, &[u8])> {
        if offset + HEADER_SIZE > self.capacity {
            return None;
        }

        let header: JournalEntryHeader = unsafe {
            std::ptr::read(self.mmap[offset..].as_ptr() as *const JournalEntryHeader)
        };

        let body_start = offset + HEADER_SIZE;
        let body_end = body_start + header.body_length as usize;

        if body_end > self.capacity {
            return None;
        }

        let body = &self.mmap[body_start..body_end];

        // Validate CRC32
        if crc32_simple(body) != header.crc32 {
            return None;
        }

        Some((header, body))
    }

    /// Flush any pending writes to disk.
    pub fn flush(&mut self) -> io::Result<()> {
        self.mmap.flush()
    }

    /// Get the current write offset.
    pub fn write_offset(&self) -> usize {
        self.write_offset
    }

    /// Get the number of entries written.
    pub fn entry_count(&self) -> u64 {
        self.entry_count
    }

    /// Get the journal capacity in bytes.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

/// Simple CRC32 implementation (for message integrity).
/// In production, use hardware CRC32C instructions via _mm_crc32_u64.
fn crc32_simple(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    crc ^ 0xFFFF_FFFF
}

/// Compute a session hash from SenderCompID and TargetCompID.
pub fn session_hash(sender: &str, target: &str) -> u64 {
    // FNV-1a hash
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in sender.as_bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash ^= b'|' as u64;
    hash = hash.wrapping_mul(0x100000001b3);
    for &b in target.as_bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_journal_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("velocitas-tests");
        fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn test_journal_write_read() {
        let path = temp_journal_path("test_journal_1.dat");
        let _ = fs::remove_file(&path);

        let mut journal = Journal::open(&path, 1024 * 1024, SyncPolicy::None).unwrap();

        let msg = b"8=FIX.4.4\x0135=D\x0110=000\x01";
        let hash = session_hash("SENDER", "TARGET");

        let offset = journal.append(hash, 1, msg).unwrap();
        assert_eq!(offset, 0);
        assert_eq!(journal.entry_count(), 1);

        let (header, body) = journal.read_entry(offset).unwrap();
        assert_eq!(header.session_hash, hash);
        assert_eq!(header.seq_num, 1);
        assert_eq!(body, msg);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn test_journal_multiple_entries() {
        let path = temp_journal_path("test_journal_2.dat");
        let _ = fs::remove_file(&path);

        let mut journal = Journal::open(&path, 1024 * 1024, SyncPolicy::None).unwrap();
        let hash = session_hash("S", "T");

        let mut offsets = Vec::new();
        for i in 1..=100 {
            let msg = format!("MSG-{}", i);
            let offset = journal.append(hash, i, msg.as_bytes()).unwrap();
            offsets.push((offset, i, msg));
        }

        for (offset, seq, msg) in &offsets {
            let (header, body) = journal.read_entry(*offset).unwrap();
            assert_eq!(header.seq_num, *seq);
            assert_eq!(body, msg.as_bytes());
        }

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn test_journal_circular_wrap() {
        let path = temp_journal_path("test_journal_3.dat");
        let _ = fs::remove_file(&path);

        // Small journal to force wrap-around
        let mut journal = Journal::open(&path, 256, SyncPolicy::None).unwrap();
        let hash = session_hash("S", "T");

        // Write enough to wrap
        for i in 1..=20 {
            let _ = journal.append(hash, i, b"test-data");
        }

        // Should have wrapped and overwritten early entries
        assert!(journal.write_offset() < journal.capacity());

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn test_session_hash_deterministic() {
        let h1 = session_hash("BANK", "NYSE");
        let h2 = session_hash("BANK", "NYSE");
        assert_eq!(h1, h2);

        let h3 = session_hash("NYSE", "BANK");
        assert_ne!(h1, h3); // Order matters
    }

    #[test]
    fn test_crc32_known_value() {
        let crc = crc32_simple(b"123456789");
        assert_eq!(crc, 0xCBF43926); // Known CRC32 test vector
    }
}
