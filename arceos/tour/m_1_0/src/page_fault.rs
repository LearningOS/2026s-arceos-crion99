#![allow(dead_code)]

use axhal::trap::{register_trap_handler, PAGE_FAULT};
use memory_addr::VirtAddr;
use page_table_entry::MappingFlags;
use axtask::TaskExtRef;
#[register_trap_handler(PAGE_FAULT)]
fn handle_page_fault(vaddr: VirtAddr, access_flags: MappingFlags, is_user: bool) -> bool {
    ax_println!(
        "handle_page_fault: vaddr={:?}, access_flags={:?}, is_user={}",
        vaddr,
        access_flags,
        is_user
    );

    if !is_user {
        return false;
    }

    let curr = axtask::current();

    let mut aspace = curr.task_ext().aspace.lock();

    aspace.handle_page_fault(vaddr, access_flags)
}