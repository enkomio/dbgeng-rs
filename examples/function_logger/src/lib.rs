mod logger;
mod bp;
mod cmd;

use std::sync::Once;
use dbgeng::client::DebugClient;
use dbgeng::export_cmd;
use windows::core::HRESULT;
use windows::Win32::Foundation::S_OK;
use windows::Win32::System::Diagnostics::Debug::Extensions::{
    DEBUG_CLASS_KERNEL, DEBUG_CLASS_USER_WINDOWS
};
use windows::Win32::System::SystemInformation::IMAGE_FILE_MACHINE_AMD64;
use logger::CLIENT;
use bp::BREAKPOINTS;
use cmd::log_function;

#[export_name = "DebugExtensionInitialize"]
fn initialize(version: *mut u32, flags: *mut u32) -> HRESULT {
    unsafe {
        *version = 0x0001_0000;
        *flags = 0x00000000;
    }

    static INIT_ONCE: Once = Once::new();
    INIT_ONCE.call_once(|| {
        let client = DebugClient::create().unwrap();

        // Let's make sure this is a live debugging, not a dump, etc..
        let is_live = matches!(
            client.debuggee_type().unwrap(),
            (DEBUG_CLASS_KERNEL | DEBUG_CLASS_USER_WINDOWS, _)
        );

        let is_intel64 = matches!(
            client.processor_type().unwrap(), 
            IMAGE_FILE_MACHINE_AMD64
        );

        if !is_intel64 {
            let _ = dbgeng::dlogln!(client, "expected an Intel 64-bit guest target");
        }
        else if !is_live {
            let _ = dbgeng::dlogln!(client, "Expecting a live debugging session");
        }
        else {
            // If we fail to create the client here, we're boned.                
            if let Err(e) = logger::init_accessible(client.clone()) {
                let _ = dbgeng::dlogln!(client, "Failed to initialize the extension: {e}");
            }                        
        }
    });
    S_OK
}

#[export_name = "DebugExtensionUninitialize"]
fn uninitialize() {
    CLIENT.with(|c| {        
        if let Some(client) = c.get() {            
            BREAKPOINTS.with(|breakpoints| {
                breakpoints.uninitialize(client);
            });
            let _ = drop(client);
        }      
    });    
}

export_cmd!(lf, log_function);
export_cmd!(logfunction, log_function);