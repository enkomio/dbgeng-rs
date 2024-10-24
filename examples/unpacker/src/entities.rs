use std::{collections::HashMap, path::PathBuf};
use dbgeng::{breakpoint::DebugBreakpoint, client::DebugClient};
use windows::core::GUID;

pub struct  AllocatedMemory {
    pub size: u64,
    pub protection: u32,
    pub returned_address: u64
}

pub struct CallbackBreakpointData {
    pub function_exit_hooked: bool,
    bp: DebugBreakpoint,
    pub allocation: Option<AllocatedMemory>
}

#[derive(Default)]
pub struct MemoryRegions {
    file: PathBuf,
    breakpoints: HashMap<GUID, CallbackBreakpointData>
}

impl MemoryRegions {
    pub fn set_file(&mut self, file: &PathBuf) {
        self.file = file.clone();
    }

    pub fn add_breakpoint(&mut self, bp: DebugBreakpoint) {
        self.breakpoints.insert(
            bp.guid().unwrap(), CallbackBreakpointData {
                function_exit_hooked: false,
                bp,
                allocation: None
            });
    }

    pub fn add_breakpoint_with_allocation(&mut self, bp: DebugBreakpoint, mem_allocation: AllocatedMemory) {
        self.breakpoints.insert(
            bp.guid().unwrap(), CallbackBreakpointData {
                function_exit_hooked: false,
                bp,
                allocation: Some(mem_allocation)
            });
    }

    pub fn function_exit_hooked(&mut self, bp: &DebugBreakpoint) {
        self.breakpoints.entry(bp.guid().unwrap()).and_modify(|bpd| bpd.function_exit_hooked = true);
    }

    pub fn is_function_exit_hooked(&self, bp: &DebugBreakpoint) -> bool {
        if let Some(bp) = self.breakpoints.get(&bp.guid().unwrap()) 
        { bp.function_exit_hooked } else { false}

    }

    pub fn get_breakpoint(&mut self, bp: &DebugBreakpoint) -> Option<&mut CallbackBreakpointData> {
        self.breakpoints.get_mut(&bp.guid().unwrap())
    }

    pub fn get_allocation(&self, address: u64) -> Option<&AllocatedMemory> {
        self.breakpoints.values()
        .find_map(|bp| { 
            if let Some(alloc) = &bp.allocation {
                if address >= alloc.returned_address && address < alloc.returned_address + alloc.size {
                    Some(alloc)
                }
                else { None }    
            }
            else { 
                None 
            }
        })
    }

    pub fn is_monitored_breakpoint(&mut self, bp: &DebugBreakpoint) -> bool {
        let bp_id = bp.guid().unwrap();
        self.breakpoints.iter().any(|(_,b)| b.bp.guid().unwrap() == bp_id)
    }
}

impl Drop for MemoryRegions {
    fn drop(&mut self) {
        let client = DebugClient::create().unwrap(); 
        for (_, bp) in self.breakpoints.drain() {
            let _ = client.remove_breakpoint(bp.bp);
        }
    }
}