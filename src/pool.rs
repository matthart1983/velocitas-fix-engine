/// Pre-allocated, lock-free memory pool for zero-allocation message handling.
///
/// Uses a fixed-size slab allocator with a lock-free free-list (LIFO stack).
/// All memory is pre-allocated and page-faulted at initialization to avoid
/// allocation on the hot path.
use std::sync::atomic::{AtomicU32, Ordering};

/// A handle to a pooled buffer. Index into the pool's slab array.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolHandle(u32);

impl PoolHandle {
    pub fn index(&self) -> usize {
        self.0 as usize
    }
}

/// Fixed-size buffer pool with lock-free allocation.
pub struct BufferPool {
    /// Backing memory: contiguous allocation of `capacity * slot_size` bytes.
    storage: Vec<u8>,
    /// Size of each slot in bytes.
    slot_size: usize,
    /// Total number of slots.
    capacity: u32,
    /// Lock-free free-list implemented as an indexed stack.
    /// Each entry contains the index of the next free slot (or u32::MAX for end).
    free_list: Vec<AtomicU32>,
    /// Head of the free list (index of first free slot).
    free_head: AtomicU32,
}

const END_OF_LIST: u32 = u32::MAX;

impl BufferPool {
    /// Create a new buffer pool with `capacity` slots of `slot_size` bytes each.
    /// All memory is pre-allocated and touched to ensure pages are resident.
    pub fn new(slot_size: usize, capacity: u32) -> Self {
        let total_bytes = slot_size * capacity as usize;
        let mut storage = vec![0u8; total_bytes];

        // Touch every page to force physical allocation (avoid page faults on hot path)
        let page_size = 4096;
        for i in (0..total_bytes).step_by(page_size) {
            storage[i] = 0;
        }

        // Initialize free list: each slot points to the next
        let free_list: Vec<AtomicU32> = (0..capacity)
            .map(|i| {
                if i + 1 < capacity {
                    AtomicU32::new(i + 1)
                } else {
                    AtomicU32::new(END_OF_LIST)
                }
            })
            .collect();

        BufferPool {
            storage,
            slot_size,
            capacity,
            free_list,
            free_head: AtomicU32::new(0),
        }
    }

    /// Allocate a buffer from the pool. Returns `None` if the pool is exhausted.
    /// This is lock-free and wait-free for the common case.
    #[inline]
    pub fn allocate(&self) -> Option<PoolHandle> {
        loop {
            let head = self.free_head.load(Ordering::Acquire);
            if head == END_OF_LIST {
                return None; // Pool exhausted
            }

            let next = self.free_list[head as usize].load(Ordering::Relaxed);
            match self.free_head.compare_exchange_weak(
                head,
                next,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Some(PoolHandle(head)),
                Err(_) => continue, // CAS failed, retry
            }
        }
    }

    /// Return a buffer to the pool. The handle must have been previously allocated.
    #[inline]
    pub fn deallocate(&self, handle: PoolHandle) {
        debug_assert!((handle.0) < self.capacity);
        loop {
            let head = self.free_head.load(Ordering::Acquire);
            self.free_list[handle.0 as usize].store(head, Ordering::Relaxed);
            match self.free_head.compare_exchange_weak(
                head,
                handle.0,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(_) => continue,
            }
        }
    }

    /// Get a mutable slice to the buffer at the given handle.
    #[inline]
    pub fn get_mut(&mut self, handle: PoolHandle) -> &mut [u8] {
        let offset = handle.0 as usize * self.slot_size;
        &mut self.storage[offset..offset + self.slot_size]
    }

    /// Get an immutable slice to the buffer at the given handle.
    #[inline]
    pub fn get(&self, handle: PoolHandle) -> &[u8] {
        let offset = handle.0 as usize * self.slot_size;
        &self.storage[offset..offset + self.slot_size]
    }

    /// Returns the slot size in bytes.
    pub fn slot_size(&self) -> usize {
        self.slot_size
    }

    /// Returns the total capacity (number of slots).
    pub fn capacity(&self) -> u32 {
        self.capacity
    }
}

/// Tiered buffer pool with small, medium, and large buffers.
pub struct TieredPool {
    pub small: BufferPool,  // ≤ 256 bytes
    pub medium: BufferPool, // ≤ 4 KB
    pub large: BufferPool,  // ≤ 64 KB
}

impl TieredPool {
    pub fn new() -> Self {
        TieredPool {
            small: BufferPool::new(256, 1_048_576), // 256 MB
            medium: BufferPool::new(4096, 262_144), // 1 GB
            large: BufferPool::new(65_536, 16_384), // 1 GB
        }
    }

    /// Allocate a buffer large enough for `size` bytes.
    pub fn allocate(&self, size: usize) -> Option<(PoolHandle, &BufferPool)> {
        if size <= 256 {
            self.small.allocate().map(|h| (h, &self.small))
        } else if size <= 4096 {
            self.medium.allocate().map(|h| (h, &self.medium))
        } else if size <= 65_536 {
            self.large.allocate().map(|h| (h, &self.large))
        } else {
            None // Message too large
        }
    }
}

impl Default for TieredPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocate_and_deallocate() {
        let mut pool = BufferPool::new(256, 16);

        let h1 = pool.allocate().expect("should allocate");
        let h2 = pool.allocate().expect("should allocate");
        assert_ne!(h1, h2);

        // Write to buffer
        let buf = pool.get_mut(h1);
        buf[0] = 42;
        assert_eq!(pool.get(h1)[0], 42);

        // Deallocate and reallocate
        pool.deallocate(h1);
        let h3 = pool.allocate().expect("should allocate recycled slot");
        assert_eq!(h3, h1); // LIFO: should get the same slot back
    }

    #[test]
    fn test_pool_exhaustion() {
        let pool = BufferPool::new(64, 2);

        let _h1 = pool.allocate().expect("first alloc");
        let _h2 = pool.allocate().expect("second alloc");
        assert!(pool.allocate().is_none(), "pool should be exhausted");
    }

    #[test]
    fn test_allocate_all_and_return_all() {
        let pool = BufferPool::new(64, 100);
        let mut handles = Vec::new();

        for _ in 0..100 {
            handles.push(pool.allocate().expect("should allocate"));
        }
        assert!(pool.allocate().is_none());

        for h in handles {
            pool.deallocate(h);
        }

        // Should be able to allocate all again
        for _ in 0..100 {
            pool.allocate().expect("should allocate after dealloc");
        }
    }
}
