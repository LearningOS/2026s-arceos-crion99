#![no_std]

use allocator::{AllocError, BaseAllocator, ByteAllocator, PageAllocator};
use core::alloc::Layout;
use core::ptr::NonNull;
/// Early memory allocator
/// Use it before formal bytes-allocator and pages-allocator can work!
/// This is a double-end memory range:
/// - Alloc bytes forward
/// - Alloc pages backward
///
/// [ bytes-used | avail-area | pages-used ]
/// |            | -->    <-- |            |
/// start       b_pos        p_pos       end
///
/// For bytes area, 'count' records number of allocations.
/// When it goes down to ZERO, free bytes-used area.
/// For pages area, it will never be freed!
///
///两个对齐函数
#[inline]
const fn align_down(pos: usize, align: usize) -> usize {
    pos & !(align - 1)
}

#[inline]
const fn align_up(pos: usize, align: usize) -> usize {
    (pos + align - 1) & !(align - 1)
}
pub struct EarlyAllocator<const SIZE: usize> {
    start: usize,
    end: usize,
    b_pos: usize,
    p_pos: usize,
    count: usize,
}

impl<const SIZE: usize> EarlyAllocator<SIZE> {
    pub const fn new() -> Self {
        Self {
            start: 0,
            end: 0,
            b_pos: 0,
            p_pos: 0,
            count: 0,
        }
    }
    #[inline]
fn is_empty(&self) -> bool {
    self.start == self.end
}

}

impl<const SIZE: usize> BaseAllocator for EarlyAllocator<SIZE> {
    fn init(&mut self, start: usize, size: usize) {
        self.start = start;
        self.end = start + size;
        self.b_pos = start;
        self.p_pos = start + size;
        self.count = 0;
    }

    fn add_memory(&mut self, start: usize, size: usize) -> allocator::AllocResult {
        if size == 0 {
            return Err(AllocError::InvalidParam);
        }

        if self.is_empty() {
            self.init(start, size);
            Ok(())
        } else {
            Err(AllocError::NoMemory)
        }
    }
}

impl<const SIZE: usize> ByteAllocator for EarlyAllocator<SIZE> {
    fn alloc(
        &mut self,
        layout: core::alloc::Layout,
    ) -> allocator::AllocResult<core::ptr::NonNull<u8>> {
        let size = layout.size();
        let align = layout.align();

        if size == 0 {
            return Ok(NonNull::dangling());
        }

        if !align.is_power_of_two() {
            return Err(AllocError::InvalidParam);
        }


        let alloc_start = align_up(self.b_pos, align);
        let alloc_end = alloc_start.checked_add(size).ok_or(AllocError::NoMemory)?;

        // 不能撞到右侧的页分配区。
        if alloc_end > self.p_pos {
            return Err(AllocError::NoMemory);
        }

        let ptr = NonNull::new(alloc_start as *mut u8).ok_or(AllocError::InvalidParam)?;

        self.b_pos = alloc_end;
        self.count += 1;

        Ok(ptr)
    }

    fn dealloc(&mut self, pos: core::ptr::NonNull<u8>, layout: core::alloc::Layout) {
        if layout.size() == 0 {
            return;
        }

        if self.count == 0 {
            return;
        }

        self.count -= 1;


        if self.count == 0 {
            self.b_pos = self.start;
        }
    }

    fn total_bytes(&self) -> usize {
         self.end - self.start
    }

    fn used_bytes(&self) -> usize {
        self.b_pos - self.start
    }

    fn available_bytes(&self) -> usize {
        self.p_pos - self.b_pos
    }
}

impl<const SIZE: usize> PageAllocator for EarlyAllocator<SIZE> {
    const PAGE_SIZE: usize = SIZE;

    fn alloc_pages(
        &mut self,
        num_pages: usize,
        align_pow2: usize,
    ) -> allocator::AllocResult<usize> {
          if num_pages == 0 || align_pow2 == 0 || !align_pow2.is_power_of_two() {
            return Err(AllocError::InvalidParam);
        }

        let size = num_pages
            .checked_mul(Self::PAGE_SIZE)
            .ok_or(AllocError::InvalidParam)?;

        // align_pow2 表示按多少页对齐。
        // 例如 PAGE_SIZE=4096，align_pow2=2，
        // 那么地址需要按 8192 字节对齐。
        let align = align_pow2
            .checked_mul(Self::PAGE_SIZE)
            .ok_or(AllocError::InvalidParam)?;

        // 页分配从 p_pos 向左增长。
        let raw_start = self.p_pos
            .checked_sub(size)
            .ok_or(AllocError::NoMemory)?;

        let alloc_start = align_down(raw_start, align);
        let alloc_end = alloc_start
            .checked_add(size)
            .ok_or(AllocError::NoMemory)?;

        // 不能越过当前 p_pos，也不能撞到左边的字节分配区。
        if alloc_end > self.p_pos || alloc_start < self.b_pos {
            return Err(AllocError::NoMemory);
        }

        self.p_pos = alloc_start;

        Ok(alloc_start)
    }

    fn dealloc_pages(&mut self, pos: usize, num_pages: usize) {
        
    }

    fn total_pages(&self) -> usize {
        self.total_bytes() / Self::PAGE_SIZE
    }

    fn used_pages(&self) -> usize {
          (self.end - self.p_pos) / Self::PAGE_SIZE
    }

    fn available_pages(&self) -> usize {
         (self.p_pos - self.b_pos) / Self::PAGE_SIZE
    }
}
