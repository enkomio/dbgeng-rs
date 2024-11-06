use std::cell::RefCell;
use std::ffi::c_void;
use std::fs;
use std::path::PathBuf;
use anyhow::{bail, Context};
use dbgeng::breakpoint::BreakpointFlags;
use dbgeng::{
    breakpoint::{BreakpointType, DebugBreakpoint}, 
    client::DebugClient, 
    events::{DebugInstruction, EventCallbacks}, 
    exception::ExceptionInfo
};
use windows::Win32::System::Diagnostics::Debug::Extensions::{DEBUG_CES_EXECUTION_STATUS, DEBUG_STATUS_BREAK};
use windows::Win32::System::Memory::{VirtualProtectEx, PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS, PAGE_READWRITE};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_ALL_ACCESS};
use windows::Win32::Foundation::{CloseHandle, EXCEPTION_ACCESS_VIOLATION};

use crate::entities::{AllocatedMemory, BreakpointFunction, MemoryRegions};

thread_local! {
    pub static MEMORY_REGIONS: MemoryRegions = MemoryRegions::new();
}

fn dump_dynamic_code(client: &DebugClient, mem_alloc: &AllocatedMemory) -> anyhow::Result<()> {
    MEMORY_REGIONS.with(|regions| { 
        let file_name = regions.get_dump_file(mem_alloc)?;
        if !file_name.is_file() {
            let mut buffer = vec![0; mem_alloc.size as usize];
            client.read_virtual(mem_alloc.address, &mut buffer[..])?;        
            fs::write(&file_name, buffer)?;
            dbgeng::dlogln!(client, "Dumped allocated memory to file: {}", file_name.display())?;            
        }
        Ok(())
    })    
}

#[allow(non_snake_case)]
fn VirtualAlloc_exit(regions: &MemoryRegions, client: &DebugClient) -> anyhow::Result<()> {
    let regs = client.regs64(&["rax", "rip"])?;
    let rax = regs[0];
    let rip = regs[1];
    let allocation_size = regions.update_allocation(rip, rax);
    let _ = dbgeng::dlogln!(client, "Allocated 0x{:x} bytes at address: 0x{:x}", allocation_size, rax);
    Ok(())
}

#[allow(non_snake_case)]
fn VirtualAlloc_enter(regions: &MemoryRegions, client: &DebugClient, bp: &DebugBreakpoint) -> anyhow::Result<()> { 
    let regs = client.regs64(&["rdx", "r9"])?;
    let stack = client.context_stack_frames(1).unwrap();
    let ro = stack[0].ReturnOffset;  
    let _ = dbgeng::dlogln!(client, "Requested allocation for 0x{:x} bytes with protection 0x{:x}", regs[0], regs[1]);      

    // create new allocation
    let allocation = AllocatedMemory {        
        size: regs[0],
        protection: regs[1] as u32,
        address: 0,
        function_return: ro
    };
    regions.new_allocation(&allocation);    
        
    // set a bp on the return address if necessary
    if !regions.is_function_exit_hooked(ro) {        
        let bp_exit = client.add_breakpoint(BreakpointType::Code, None).unwrap();
        let _ = bp_exit.set_offset(ro);
        let _ = bp_exit.set_flags(BreakpointFlags::ENABLED);
        regions.add_breakpoint(bp_exit, ro, BreakpointFunction::VirtualAllocExit);
        regions.set_function_exit_hooked(bp);
        let _ = dbgeng::dlogln!(client, "Hook VirtualAlloc return address at 0x{:x}", ro);
    }    

    // write PAGE_READWRITE if PAGE_EXECUTE_READWRITE is found
    if regs[1] as u32 == PAGE_EXECUTE_READWRITE.0 {
        let _ = client.set_reg64("r9", PAGE_READWRITE.0 as u64);
    }
    Ok(())
}

#[allow(non_snake_case)]
fn VirtualFree(regions: &MemoryRegions, client: &DebugClient) -> anyhow::Result<()> {
    let regs = client.regs64(&["rcx", "rdx"])?;
    regions.free_allocation(regs[0], regs[1]);
    Ok(())
}

fn handle_exception(client: &DebugClient, ei: &ExceptionInfo) -> anyhow::Result<()> {
    MEMORY_REGIONS.with(|regions| { 
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
                dump_dynamic_code(client, &mem_alloc)?;
                
                // set back the original protection
                let mut old_protect = PAGE_PROTECTION_FLAGS::default();
                let address = mem_alloc.address as *const c_void;
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
            }                          
        }  
        Ok(())
    })

}

pub fn start_monitor(_: &DebugClient, args: String) -> anyhow::Result<()> {
    let client = DebugClient::create().unwrap();  
    let mut args = args.split_whitespace();
    let directory = PathBuf::from(args.next().context("missing directory name")?.to_string());
    if !directory.is_dir() {
        fs::create_dir_all(&directory)?;
    }

    MEMORY_REGIONS.with(|regions| { 
        regions.set_directory(&directory);
        
        let bp = client.add_breakpoint(BreakpointType::Code, None)?;
        bp.set_offset_expression(String::from("KERNELBASE!VirtualAlloc"))?;
        bp.set_flags(BreakpointFlags::ENABLED)?;
        regions.add_breakpoint(bp, 0, BreakpointFunction::VirtualAllocEnter);
        let _ = dbgeng::dlogln!(client, "Added KERNELBASE!VirtualAlloc for monitoring memory allocation");        
        
        let bp_free = client.add_breakpoint(BreakpointType::Code, None)?;
        bp_free.set_offset_expression(String::from("KERNELBASE!VirtualFree"))?;
        bp_free.set_flags(BreakpointFlags::ENABLED)?;
        regions.add_breakpoint(bp_free, 0, BreakpointFunction::VirtualFree);
        let _ = dbgeng::dlogln!(client, "Added KERNELBASE!VirtualFree for monitoring memory deallocation");        
        
        client.set_event_callbacks(PluginEventCallbacks {exception_handled: RefCell::new(0)})
    })
}

fn handle_breakpoint(client: &DebugClient, bp: &DebugBreakpoint) {
    MEMORY_REGIONS.with(|regions| {        
        match regions.get_breakpoint_type(bp) {
            BreakpointFunction::VirtualAllocEnter => { let _ = VirtualAlloc_enter(regions, client, bp); },
            BreakpointFunction::VirtualAllocExit => { let _ = VirtualAlloc_exit(regions, client); },
            BreakpointFunction::VirtualFree => { let _ = VirtualFree(regions, client); },
            _ => {}
        }
    });
}

struct PluginEventCallbacks {
    exception_handled: RefCell<i32>
}

impl EventCallbacks for PluginEventCallbacks {
    fn breakpoint(&self, client: &DebugClient, bp: &DebugBreakpoint) -> DebugInstruction {        
        if MEMORY_REGIONS.with(|regions| regions.is_monitored_breakpoint(bp)) {
            handle_breakpoint(client, bp);
            DebugInstruction::Go
        }
        else {
            DebugInstruction::NoChange
        }
    }

    fn exception(&self, client: &DebugClient, ei: &ExceptionInfo) -> DebugInstruction {    
        if ei.record.exception_code == EXCEPTION_ACCESS_VIOLATION {            
            let _ = dbgeng::dlogln!(client, 
                "Exception at 0x{:x} first chance: {}. Exception type: 0x{:x}", 
                ei.record.exception_address, 
                ei.first_chance, 
                ei.record.exception_code.0 as u32
            );           

            match handle_exception(client, ei) {
                Err(e) => { let _ = dbgeng::dlogln!(client, "Error during exception handling for created breakpoint: {e}"); },
                Ok(_) => {
                    self.exception_handled.replace(2);
                    return DebugInstruction::GoHandled;
                }
            }
        }
        DebugInstruction::GoNotHandled        
    }

    fn change_engine_state(&self, client: &DebugClient, flags: u32, argument: u64) {  
        let exception_pending = self.exception_handled.borrow().is_positive();
        if flags == DEBUG_CES_EXECUTION_STATUS && argument as u32 == DEBUG_STATUS_BREAK && exception_pending {
            let old_value = self.exception_handled.replace_with(|&mut old| old - 1);        
            if old_value > 0 {
                let _ = dbgeng::dlogln!(client, "Continue execution with 'g'");
                let _ = client.exec("g");
            }
        } 
    }
}