use anyhow::{Context, Result};
use dbgeng::breakpoint::{BreakpointFlags, BreakpointType};
use dbgeng::client::DebugClient;
use dbgeng::events::DebugInstruction;
use crate::logger;
use crate::bp::BREAKPOINTS;

pub fn log_function(_: &DebugClient, args: String) -> anyhow::Result<()> {
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