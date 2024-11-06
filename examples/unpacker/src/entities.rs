use std::{cell::RefCell, collections::HashMap, path::{self, PathBuf}};
use anyhow;
use dbgeng::{breakpoint::DebugBreakpoint, client::DebugClient};
use windows::core::GUID;

#[derive(Clone)]
pub struct  AllocatedMemory {
    pub size: u64,
    pub protection: u32,
    pub address: u64,
    pub function_return: u64
}

#[derive(Clone, Copy, PartialEq)]
pub enum BreakpointFunction {
    VirtualAllocEnter,
    VirtualAllocExit,
    VirtualFree,
    None
}

pub struct CallbackBreakpointData {
    address: u64,
    function: BreakpointFunction,
    bp: DebugBreakpoint,    
}

#[derive(Default)]
pub struct MemoryRegions {
    directory: RefCell<PathBuf>,
    breakpoints: RefCell<HashMap<GUID, CallbackBreakpointData>>,
    allocations: RefCell<Vec<AllocatedMemory>>
}

impl MemoryRegions {
    pub fn new() -> MemoryRegions {
        MemoryRegions {
            directory: RefCell::new(PathBuf::new()),
            breakpoints: RefCell::new(HashMap::new()),
            allocations: RefCell::new(Vec::new())
        }
    }

    pub fn set_directory(&self, directory: &PathBuf) {
        *self.directory.borrow_mut() = directory.clone();
    }

    pub fn get_dump_file(&self, mem_alloc: &AllocatedMemory) -> anyhow::Result<PathBuf> {
        let file_name = self.directory.borrow().join(format!("dump_{:x}_{}.bin", mem_alloc.address, mem_alloc.size));
        path::absolute(file_name).map_err(anyhow::Error::from)
    }

    pub fn add_breakpoint(&self, bp: DebugBreakpoint, address: u64, function: BreakpointFunction) {
        self.breakpoints.borrow_mut().insert(
            bp.guid().unwrap(), CallbackBreakpointData {
                function,
                address,
                bp
            });
    }

    pub fn is_monitored_breakpoint(&self, bp: &DebugBreakpoint) -> bool {
        let bp_id = bp.guid().unwrap();
        self.breakpoints.borrow().iter().any(|(_,b)| b.bp.guid().unwrap() == bp_id)
    }

    pub fn is_address_hooked(&self, address: u64, bp_type: BreakpointFunction) -> bool {
        self.breakpoints.borrow().values().any(|bp| bp.function == bp_type && bp.address == address)
    }

    pub fn remove_all_breakpoints(&self) {
        self.breakpoints.borrow_mut().drain();
    }

    pub fn get_breakpoint_type(&self, bp: &DebugBreakpoint) -> BreakpointFunction {
        match self.breakpoints.borrow().get(&bp.guid().unwrap()) {
            Some(bpd) => bpd.function,
            _ => BreakpointFunction::None
        }
    }

    pub fn new_allocation(&self, mem_allocation: &AllocatedMemory) {
        self.allocations.borrow_mut().push(mem_allocation.clone());
    }

    pub fn update_allocation(&self, function_return_addr: u64, allocated_address: u64) -> u64 {
        if let Some(allocation) = self.allocations.borrow_mut().iter_mut().find(|a| a.function_return == function_return_addr) {
            allocation.address = allocated_address;
            allocation.size
        }
        else {
            0
        }
    }

    pub fn get_allocation(&self, address: u64) -> Option<AllocatedMemory> {
        self.allocations.borrow().iter().find(|a| address >= a.address && a.address + a.size < address).cloned()
    }

    pub fn free_allocation(&self, address: u64, size: u64) {
        let mut allocs = self.allocations.borrow_mut();
        if let Some(index) = allocs.iter().position(|a| a.address == address && (a.size == size || size == 0)) {
            allocs.remove(index);
        }
    }
}

impl Drop for MemoryRegions {
    fn drop(&mut self) {
        let client = DebugClient::create().unwrap(); 
        for (_, bp) in self.breakpoints.get_mut().drain() {
            let _ = client.remove_breakpoint(bp.bp);
        }
    }
}