use std::cell::{OnceCell, RefCell};
use std::ffi::c_void;
use std::path::PathBuf;
use anyhow::{bail, Context};
use dbgeng::breakpoint::BreakpointFlags;
use dbgeng::{
    breakpoint::{BreakpointType, DebugBreakpoint}, 
    client::DebugClient, 
    events::{DebugInstruction, EventCallbacks}, 
    exception::ExceptionInfo
};
use windows::Win32::System::Memory::{VirtualProtectEx, PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS, PAGE_READWRITE};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_ALL_ACCESS};
use windows::Win32::Foundation::{CloseHandle, EXCEPTION_ACCESS_VIOLATION};

use crate::entities::{AllocatedMemory, MemoryRegions};

thread_local! {
    pub static MEMORY_REGIONS: OnceCell<RefCell<MemoryRegions>> = OnceCell::new();
}

#[allow(non_snake_case)]
fn VirtualAlloc_exit(regions: &mut MemoryRegions, client: &DebugClient, bp: &DebugBreakpoint) -> anyhow::Result<()> {
    let rax = client.reg64("rax")?;
    if let Some(bp_data) = regions.get_breakpoint(bp) {
        if let Some(allocation) = bp_data.allocation.as_mut() {
            allocation.returned_address = rax;
        }        
    }
    Ok(())
}

#[allow(non_snake_case)]
fn VirtualAlloc_enter(regions: &mut MemoryRegions, client: &DebugClient, bp: &DebugBreakpoint) -> anyhow::Result<()> { 
    let regs = client.regs64(&["rdx", "r9"])?;
    let allocation = AllocatedMemory {        
        size: regs[0],
        protection: regs[1] as u32,
        returned_address: 0
    };
        
    // set a bp on the return address if necessary
    if !regions.is_function_exit_hooked(bp) {
        let stack = client.context_stack_frames(1).unwrap();
        let ro = stack[0].ReturnOffset;        
        let bp_exit = client.add_breakpoint(BreakpointType::Code, None).unwrap();
        let _ = bp_exit.set_offset(ro);
        let _ = bp_exit.set_flags(BreakpointFlags::ENABLED);
        regions.add_breakpoint_with_allocation(bp_exit, allocation);
        regions.function_exit_hooked(bp);
        let _ = dbgeng::dlogln!(client, "Hook VirtualAlloc return address at 0x{:x}", ro);
    }

    // write PAGE_READWRITE if PAGE_EXECUTE_READWRITE is found
    if regs[1] as u32 == PAGE_EXECUTE_READWRITE.0 {
        let _ = client.set_reg64("r9", PAGE_READWRITE.0 as u64);
    }
    Ok(())
}

fn handle_exception(client: &DebugClient, ei: &ExceptionInfo) -> anyhow::Result<()> {
    MEMORY_REGIONS.with(|regions| { 
        let regions = regions.get().context("regions object not set")?.borrow_mut();
        if let Some(mem_alloc) = regions.get_allocation(ei.record.exception_address) {
            let pid = client.get_current_process_id()?;
            let process_handle = unsafe {
                OpenProcess(
                    PROCESS_ALL_ACCESS, 
                    false, 
                    pid
                )?
            };

            if process_handle.is_invalid() {
                bail!("Unable to ope the process {pid}")
            }
            else {
                // set back the original protection
                let mut old_protect = PAGE_PROTECTION_FLAGS::default();
                let address = mem_alloc.returned_address as *const c_void;
                unsafe {
                    VirtualProtectEx(
                        process_handle, 
                        address, 
                        mem_alloc.size as usize, 
                        PAGE_PROTECTION_FLAGS(mem_alloc.protection), 
                        &mut old_protect
                    )?;

                    CloseHandle(process_handle)?;
                }                
                Ok(())
            }            
        }
        else {
            bail!("memory region not found")
        }        
    })
}

pub fn start_monitor(client: &DebugClient, args: String) -> anyhow::Result<()> {
    let mut args = args.split_whitespace();
    let file = PathBuf::from(args.next().context("missing file name")?.to_string());

    MEMORY_REGIONS.with(|regions| { 
        let mut regions = regions.get().context("regions object not set")?.borrow_mut();
        regions.set_file(&file);

        let _ = dbgeng::dlogln!(client, "Added KERNELBASE!VirtualAlloc for monitoring memory allocation");
        let bp = client.add_breakpoint(BreakpointType::Code, None)?;
        bp.set_offset_expression(String::from("KERNELBASE!VirtualAlloc"))?;
        bp.set_flags(BreakpointFlags::ENABLED)?;
        regions.add_breakpoint(bp);
        client.set_event_callbacks(PluginEventCallbacks)
    })
}

struct PluginEventCallbacks;
impl EventCallbacks for PluginEventCallbacks {
    fn breakpoint(&self, client: &DebugClient, bp: &DebugBreakpoint) -> DebugInstruction {
        MEMORY_REGIONS.with(|regions| { 
            if let Ok(regions) = regions.get().context("regions object not set") {
                let mut regions = regions.borrow_mut();
                if regions.is_monitored_breakpoint(bp) {
                    let _ = VirtualAlloc_enter(&mut regions, client, bp);
                    let _ = VirtualAlloc_exit(&mut regions, client, bp);
                    DebugInstruction::Go
                }
                else {
                    DebugInstruction::NoChange
                }
            }
            else {
                DebugInstruction::NoChange
            }
        })
    }

    fn exception(&self, client: &DebugClient, ei: &ExceptionInfo) -> DebugInstruction {    
        if ei.first_chance == 1 && ei.record.exception_code == EXCEPTION_ACCESS_VIOLATION {            
            let _ = dbgeng::dlogln!(client, "Exception at 0x{:x} first chance: {}. Exception type: 0x{:x}", 
                ei.record.exception_address, ei.first_chance, ei.record.exception_code.0 as u32);
            match handle_exception(client, ei) {
                Err(e) => {
                    let _ = dbgeng::dlogln!(client, "Error during exception handling for created breakpoint: {e}");
                },
                Ok(_) => {
                    return DebugInstruction::GoHandled;
                }
            }
        }
        DebugInstruction::GoNotHandled        
    }
}