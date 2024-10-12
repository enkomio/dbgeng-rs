use dbgeng::breakpoint::DebugBreakpoint;
use dbgeng::events::{EventCallbacks, DebugInstruction};
use anyhow::Result;
use dbgeng::client::DebugClient;
use std::cell::OnceCell;
use crate::bp::BREAKPOINTS;

thread_local! {
    pub static CLIENT: OnceCell<DebugClient> = OnceCell::new();
}

struct PluginEventCallbacks;
impl EventCallbacks for PluginEventCallbacks {
    fn breakpoint(&self, client: &DebugClient, bp: &DebugBreakpoint) -> DebugInstruction {
        BREAKPOINTS.with(|breakpoints| breakpoints.call(client, bp))
    }
}

pub fn monitored_func_end(
    client: &DebugClient,
    function_name: String
) -> Result<DebugInstruction> {
    let rax = client.reg64("rax")?;
    dbgeng::dlogln!(client, "*** Exiting {function_name} with result: 0x{:x}", rax)?;
    Ok(DebugInstruction::Go)
}

pub fn monitored_func_start(
    client: &DebugClient,
    function_name: String,
) -> Result<DebugInstruction> {    
    // get first 4 arguments
    let regs = client.regs64_dict(&["rcx", "rdx", "r8", "r9"])?;
    let args = regs.values().map(|v| format!("0x{:x}", v)).collect::<Vec<String>>().join(", ");   
    dbgeng::dlogln!(client, "*** Enter {function_name} with arguments: {args}")?;
    Ok(DebugInstruction::Go)
}

pub fn init_accessible(client: DebugClient) -> anyhow::Result<()> {
    dbgeng::dlogln!(client, "Function Logger extension initialized")?;
    client.set_event_callbacks(PluginEventCallbacks)?;
    CLIENT.with(|c| {
        c.set(client)
            .map_err(|_e| anyhow::anyhow!("Failed to set the client"))
    })?;
    Ok(())
}