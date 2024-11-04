mod entities;
mod monitor;

use dbgeng::client::DebugClient;
use dbgeng::export_cmd;
use windows::{core::HRESULT, Win32::System::SystemInformation::IMAGE_FILE_MACHINE_AMD64};
use windows::Win32::Foundation::S_OK;
use monitor::{MEMORY_REGIONS, start_monitor};

#[export_name = "DebugExtensionInitialize"]
fn initialize(version: *mut u32, flags: *mut u32) -> HRESULT {
    let client = DebugClient::create().unwrap();  
    let is_intel64 = matches!(client.processor_type().unwrap(), IMAGE_FILE_MACHINE_AMD64);
    if !is_intel64 {
        let _ = dbgeng::dlogln!(client, "expected an Intel 64-bit guest target");
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
        regions.remove_all_breakpoints();
    });
    
}

export_cmd!(start_monitor, start_monitor);