#![allow(dead_code)]

use core::ffi::{c_void, c_char, c_int};
use axhal::arch::TrapFrame;
use axhal::trap::{register_trap_handler, SYSCALL};
use axerrno::LinuxError;
use axtask::current;
use axtask::TaskExtRef;
use axhal::paging::MappingFlags;
use arceos_posix_api as api;

const SYS_IOCTL: usize = 29;
const SYS_OPENAT: usize = 56;
const SYS_CLOSE: usize = 57;
const SYS_READ: usize = 63;
const SYS_WRITE: usize = 64;
const SYS_WRITEV: usize = 66;
const SYS_EXIT: usize = 93;
const SYS_EXIT_GROUP: usize = 94;
const SYS_SET_TID_ADDRESS: usize = 96;
const SYS_MMAP: usize = 222;
const PAGE_SIZE: usize = 0x1000;
const MMAP_BASE: usize = 0x1000_0000;

const AT_FDCWD: i32 = -100;

/// Macro to generate syscall body
///
/// It will receive a function which return Result<_, LinuxError> and convert it to
/// the type which is specified by the caller.
#[macro_export]
macro_rules! syscall_body {
    ($fn: ident, $($stmt: tt)*) => {{
        #[allow(clippy::redundant_closure_call)]
        let res = (|| -> axerrno::LinuxResult<_> { $($stmt)* })();
        match res {
            Ok(_) | Err(axerrno::LinuxError::EAGAIN) => debug!(concat!(stringify!($fn), " => {:?}"),  res),
            Err(_) => info!(concat!(stringify!($fn), " => {:?}"), res),
        }
        match res {
            Ok(v) => v as _,
            Err(e) => {
                -e.code() as _
            }
        }
    }};
}

bitflags::bitflags! {
    #[derive(Debug)]
    /// permissions for sys_mmap
    ///
    /// See <https://github.com/bminor/glibc/blob/master/bits/mman.h>
    struct MmapProt: i32 {
        /// Page can be read.
        const PROT_READ = 1 << 0;
        /// Page can be written.
        const PROT_WRITE = 1 << 1;
        /// Page can be executed.
        const PROT_EXEC = 1 << 2;
    }
}

impl From<MmapProt> for MappingFlags {
    fn from(value: MmapProt) -> Self {
        let mut flags = MappingFlags::USER;
        if value.contains(MmapProt::PROT_READ) {
            flags |= MappingFlags::READ;
        }
        if value.contains(MmapProt::PROT_WRITE) {
            flags |= MappingFlags::WRITE;
        }
        if value.contains(MmapProt::PROT_EXEC) {
            flags |= MappingFlags::EXECUTE;
        }
        flags
    }
}

bitflags::bitflags! {
    #[derive(Debug)]
    /// flags for sys_mmap
    ///
    /// See <https://github.com/bminor/glibc/blob/master/bits/mman.h>
    struct MmapFlags: i32 {
        /// Share changes
        const MAP_SHARED = 1 << 0;
        /// Changes private; copy pages on write.
        const MAP_PRIVATE = 1 << 1;
        /// Map address must be exactly as requested, no matter whether it is available.
        const MAP_FIXED = 1 << 4;
        /// Don't use a file.
        const MAP_ANONYMOUS = 1 << 5;
        /// Don't check for reservations.
        const MAP_NORESERVE = 1 << 14;
        /// Allocation is for a stack.
        const MAP_STACK = 0x20000;
    }
}

#[register_trap_handler(SYSCALL)]
fn handle_syscall(tf: &TrapFrame, syscall_num: usize) -> isize {
    ax_println!("handle_syscall [{}] ...", syscall_num);
    let ret = match syscall_num {
         SYS_IOCTL => sys_ioctl(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _) as _,
        SYS_SET_TID_ADDRESS => sys_set_tid_address(tf.arg0() as _),
        SYS_OPENAT => sys_openat(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _, tf.arg3() as _),
        SYS_CLOSE => sys_close(tf.arg0() as _),
        SYS_READ => sys_read(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _),
        SYS_WRITE => sys_write(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _),
        SYS_WRITEV => sys_writev(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _),
        SYS_EXIT_GROUP => {
            ax_println!("[SYS_EXIT_GROUP]: system is exiting ..");
            axtask::exit(tf.arg0() as _)
        },
        SYS_EXIT => {
            ax_println!("[SYS_EXIT]: system is exiting ..");
            axtask::exit(tf.arg0() as _)
        },
        SYS_MMAP => sys_mmap(
            tf.arg0() as _,
            tf.arg1() as _,
            tf.arg2() as _,
            tf.arg3() as _,
            tf.arg4() as _,
            tf.arg5() as _,
        ),
        _ => {
            ax_println!("Unimplemented syscall: {}", syscall_num);
            -LinuxError::ENOSYS.code() as _
        }
    };
    ret
}

#[allow(unused_variables)]
fn sys_mmap(
    addr: *mut c_void,
    length: usize,
    prot: i32,
    flags: i32,
    fd: i32,
    offset: isize,
) -> isize {
    // 1. 基本参数检查
    if length == 0 {
        return neg_errno(LinuxError::EINVAL);
    }

    if offset < 0 || (offset as usize) & (PAGE_SIZE - 1) != 0 {
        return neg_errno(LinuxError::EINVAL);
    }

    let map_size = match align_up_4k(length) {
        Some(size) => size,
        None => return neg_errno(LinuxError::ENOMEM),
    };

    let prot_flags = match MmapProt::from_bits(prot) {
        Some(p) => p,
        None => return neg_errno(LinuxError::EINVAL),
    };

    let mmap_flags = MmapFlags::from_bits_truncate(flags);

    // mmap 最终应该给用户的权限
    let final_flags: MappingFlags = prot_flags.into();

    // 内核要先把文件内容写进去，所以临时加 WRITE。
    // 写完后再 protect 回用户要求的权限。
    let temp_flags =
        final_flags | MappingFlags::READ | MappingFlags::WRITE | MappingFlags::USER;

    // 2. 如果不是匿名映射，就从 fd 读文件内容
    let mut data = vec![0u8; length];

    if !mmap_flags.contains(MmapFlags::MAP_ANONYMOUS) {
        if fd < 0 {
            return neg_errno(LinuxError::EBADF);
        }

        let read_result = with_file_fd(fd, |file| {
            // 这个测例 offset 是 0。
            // 这里为了稍微完整一点，offset 非 0 时先读掉 offset 字节。
            let mut skip = offset as usize;
            let mut scratch = [0u8; 512];

            while skip > 0 {
                let n = core::cmp::min(skip, scratch.len());
                let read_n = file
                    .read(&mut scratch[..n])
                    .map_err(LinuxError::from)?;

                if read_n == 0 {
                    break;
                }

                skip -= read_n;
            }

            let mut done = 0usize;
            while done < data.len() {
                let n = file
                    .read(&mut data[done..])
                    .map_err(LinuxError::from)?;

                if n == 0 {
                    break;
                }

                done += n;
            }

            Ok(())
        });

        if let Err(e) = read_result {
            return neg_errno(e);
        }
    }

    // 3. 取当前用户地址空间
    let shared_aspace = match crate::USER_ASPACE.lock().as_ref().cloned() {
        Some(aspace) => aspace,
        None => return neg_errno(LinuxError::EINVAL),
    };

    let mut aspace = shared_aspace.lock();

    // 4. 选择 mmap 返回的用户虚拟地址
    let start_vaddr = if mmap_flags.contains(MmapFlags::MAP_FIXED) {
        let fixed = VirtAddr::from(addr as usize);

        if addr.is_null() || !fixed.is_aligned_4k() {
            return neg_errno(LinuxError::EINVAL);
        }

        if !aspace.contains_range(fixed, map_size) {
            return neg_errno(LinuxError::ENOMEM);
        }

        fixed
    } else {
        let limit_start = VirtAddr::from(MMAP_BASE);
        let limit_end = aspace.end();

        let mut hint = if addr.is_null() {
            limit_start
        } else {
            VirtAddr::from(addr as usize).align_up_4k()
        };

        if hint < limit_start {
            hint = limit_start;
        }

        match aspace.find_free_area(hint, map_size, va_range!(limit_start..limit_end)) {
            Some(vaddr) => vaddr,
            None => return neg_errno(LinuxError::ENOMEM),
        }
    };

    // 5. 建立用户虚拟地址到物理页的映射
    if aspace
        .map_alloc(start_vaddr, map_size, temp_flags, true)
        .is_err()
    {
        return neg_errno(LinuxError::ENOMEM);
    }

    // 6. 把文件内容写入刚刚映射好的用户虚拟地址
    if !data.is_empty() {
        if aspace.write(start_vaddr, &data).is_err() {
            let _ = aspace.unmap(start_vaddr, map_size);
            return neg_errno(LinuxError::EFAULT);
        }
    }

    // 7. 恢复成用户 mmap 请求的权限，比如 PROT_READ 就只读
    if aspace.protect(start_vaddr, map_size, final_flags).is_err() {
        let _ = aspace.unmap(start_vaddr, map_size);
        return neg_errno(LinuxError::EINVAL);
    }

    // 8. mmap 成功返回映射起始地址
    start_vaddr.as_usize() as isize
}


fn sys_openat(dfd: c_int, fname: *const c_char, flags: c_int, mode: api::ctypes::mode_t) -> isize {
    assert_eq!(dfd, AT_FDCWD);
    api::sys_open(fname, flags, mode) as isize
}

fn sys_close(fd: i32) -> isize {
    api::sys_close(fd) as isize
}

fn sys_read(fd: i32, buf: *mut c_void, count: usize) -> isize {
    api::sys_read(fd, buf, count)
}

fn sys_write(fd: i32, buf: *const c_void, count: usize) -> isize {
    api::sys_write(fd, buf, count)
}

fn sys_writev(fd: i32, iov: *const api::ctypes::iovec, iocnt: i32) -> isize {
    unsafe { api::sys_writev(fd, iov, iocnt) }
}

fn sys_set_tid_address(tid_ptd: *const i32) -> isize {
    let curr = current();
    curr.task_ext().set_clear_child_tid(tid_ptd as _);
    curr.id().as_u64() as isize
}

fn sys_ioctl(_fd: i32, _op: usize, _argp: *mut c_void) -> i32 {
    ax_println!("Ignore SYS_IOCTL");
    0
}
