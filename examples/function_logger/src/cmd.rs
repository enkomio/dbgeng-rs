use std::sync::Once;
use anyhow::{Context, Result};
use dbgeng::breakpoint::{BreakpointFlags, BreakpointType};
use dbgeng::client::DebugClient;
use dbgeng::events::DebugInstruction;
use crate::logger;
use crate::bp::BREAKPOINTS;
use windows::Win32::System::Diagnostics::Debug::Extensions::{
    DEBUG_CLASS_KERNEL, DEBUG_CLASS_USER_WINDOWS, 
};
use windows::Win32::System::SystemInformation::IMAGE_FILE_MACHINE_AMD64;

pub fn init_once(client: &DebugClient) {
    static INIT_ONCE: Once = Once::new();
    INIT_ONCE.call_once(|| {
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
}

pub fn log_function(client: &DebugClient, args: String) -> anyhow::Result<()> {
    //init_once(client);

    let mut args = args.split_whitespace();
    let function_name = args.next().context("missing function name")?.to_string();

    logger::CLIENT.with(|c| -> anyhow::Result<()> {
        let client: &DebugClient = c.get().context("client not set")?;
        let bp = client.add_breakpoint(BreakpointType::Code, None)?;
        bp.set_offset_expression(function_name.clone())?;
        bp.set_flags(BreakpointFlags::ENABLED)?;

        let _ = dbgeng::dlogln!(client, "Start monitoring of function: {function_name}");
        BREAKPOINTS.with(|breakpoints| {
            breakpoints.insert(bp, function_name.clone(), move |client, _| -> Result<DebugInstruction> {
                logger::monitored_func_start(client, function_name.clone())
            });
        });
        Ok(())
    })?;
    Ok(())
}