#![cfg_attr(feature = "axstd", no_std)]
#![cfg_attr(feature = "axstd", no_main)]

extern crate alloc;
#[cfg(feature = "axstd")]
extern crate axstd as std;

#[macro_use]
extern crate axlog;

mod loader;
mod syscall;
mod task;
pub static USER_ASPACE: Mutex<Option<Arc<Mutex<AddrSpace>>>> = Mutex::new(None);

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use axhal::arch::UspaceContext;
use axhal::mem::VirtAddr;
use axhal::paging::MappingFlags;
use axmm::AddrSpace;
use axstd::io;
use axsync::Mutex;
use loader::load_user_app;

const USER_STACK_SIZE: usize = 0x10000;
const KERNEL_STACK_SIZE: usize = 0x40000; // 256 KiB

#[cfg_attr(feature = "axstd", no_mangle)]
fn main() {
    // A new address space for user app.
    let mut uspace = axmm::new_user_aspace().unwrap();

    // Load user app binary file into address space.
    let entry = match load_user_app("/sbin/mapfile", &mut uspace) {
        Ok(e) => e,
        Err(err) => panic!("Cannot load app! {:?}", err),
    };
    ax_println!("entry: {:#x}", entry);

    // Init user stack.
    let ustack_top = init_user_stack(&mut uspace, true).unwrap();
    ax_println!("New user address space: {:#x?}", uspace);

    // Let's kick off the user process.
    // Let's kick off the user process.
    let shared_uspace = Arc::new(Mutex::new(uspace));

    *USER_ASPACE.lock() = Some(Arc::clone(&shared_uspace));

    let user_task = task::spawn_user_task(shared_uspace, UspaceContext::new(entry, ustack_top));

    // Wait for user process to exit ...
    let exit_code = user_task.join();
    ax_println!("monolithic kernel exit [{:?}] normally!", exit_code);
}

fn init_user_stack(uspace: &mut AddrSpace, populating: bool) -> io::Result<VirtAddr> {
    let ustack_top = uspace.end();
    let ustack_vaddr = ustack_top - crate::USER_STACK_SIZE;
    ax_println!(
        "Mapping user stack: {:#x?} -> {:#x?}",
        ustack_vaddr,
        ustack_top
    );
    uspace
        .map_alloc(
            ustack_vaddr,
            crate::USER_STACK_SIZE,
            MappingFlags::READ | MappingFlags::WRITE | MappingFlags::USER,
            populating,
        )
        .unwrap();

    let app_name = "hello";
    let av = BTreeMap::new();
    let (stack_data, ustack_pointer) = kernel_elf_parser::get_app_stack_region(
        &[String::from(app_name)],
        &[],
        &av,
        ustack_vaddr,
        crate::USER_STACK_SIZE,
    );
    uspace.write(VirtAddr::from_usize(ustack_pointer), stack_data.as_slice())?;

    Ok(ustack_pointer.into())
}
