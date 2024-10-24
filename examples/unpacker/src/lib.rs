mod entities;
mod monitor;

use std::cell::RefCell;
use std::sync::Once;
use dbgeng::client::DebugClient;
use dbgeng::export_cmd;
use windows::{core::HRESULT, Win32::System::SystemInformation::IMAGE_FILE_MACHINE_AMD64};
use windows::Win32::Foundation::S_OK;
use entities::MemoryRegions;
use monitor::{start_monitor, MEMORY_REGIONS};

#[export_name = "DebugExtensionInitialize"]
fn initialize(version: *mut u32, flags: *mut u32) -> HRESULT {
    let client = DebugClient::create().unwrap();  
    let is_intel64 = matches!(client.processor_type().unwrap(), IMAGE_FILE_MACHINE_AMD64);
    if !is_intel64 {
        let _ = dbgeng::dlogln!(client, "expected an Intel 64-bit guest target");
    }
    else {
        static INIT_ONCE: Once = Once::new();
        INIT_ONCE.call_once(|| {
            if let Ok(_) = MEMORY_REGIONS.with(|regions| {  
                regions.set(RefCell::new(MemoryRegions::default()))
                    .map_err(|_e| anyhow::anyhow!("Failed to set the MemoryRegions object"))
            }) {                      
                let _ = dbgeng::dlogln!(client, "Unpacker plugin loaded");
            }
        });


        
        // REMOVE
        let _ = start_monitor(&client, "unpack.txt".to_string());
    }

    unsafe {
        *version = 0x0001_0000;
        *flags = 0x00000000;
    }
    S_OK
}

#[export_name = "DebugExtensionUninitialize"]
fn uninitialize() {
    MEMORY_REGIONS.with(|regions| { 
        if let Some(regions) = regions.get() {
            let regions = regions.borrow_mut();
            let _ = drop(regions);
        }
    })
}

export_cmd!(start_monitor, start_monitor);