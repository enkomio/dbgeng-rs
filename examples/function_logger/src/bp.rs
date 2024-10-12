use std::cell::RefCell;
use std::collections::HashMap;

use anyhow::Result;
use dbgeng::breakpoint::{BreakpointFlags, BreakpointType, DebugBreakpoint};
use dbgeng::client::DebugClient;
use dbgeng::events::DebugInstruction;
use windows::core::GUID;

use crate::logger;

thread_local! {
    pub static BREAKPOINTS: CallbackBreakpoints = CallbackBreakpoints::new();
}

struct CallbackBreakpointData {
    bp: DebugBreakpoint,
    function_name: String,
    function_return_hooked: bool,
    callback: Box<dyn FnMut(&DebugClient, &DebugBreakpoint) -> Result<DebugInstruction>>,
}

pub struct CallbackBreakpoints {
    inner: RefCell<HashMap<GUID, CallbackBreakpointData>>,
}

impl CallbackBreakpoints {
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(HashMap::new()),
        }
    }

    pub fn uninit(&self, client: &DebugClient) {
        let mut inner = self.inner.borrow_mut();
        for (_, data) in inner.drain() {            
            let _ = client.remove_breakpoint(data.bp);
        }
    }

    pub fn insert<
        T: FnMut(&DebugClient, &DebugBreakpoint) -> Result<DebugInstruction> + 'static,
    >(
        &self,
        bp: DebugBreakpoint,
        function_name: String,
        cb: T,
    ) -> bool {
        self.inner
            .borrow_mut()
            .insert(bp.guid().unwrap(), CallbackBreakpointData {
                bp,
                function_name,
                function_return_hooked: false,
                callback: Box::new(cb),
            }).is_some()
    }

    pub fn call(&self, client: &DebugClient, bp: &DebugBreakpoint) -> DebugInstruction {
        let mut function_name = String::new();
        let mut need_to_hook = false;

        let mut inner = self.inner.borrow_mut();
        let result =
            if let Some(data) = inner.get_mut(&bp.guid().unwrap()) {
                need_to_hook = !data.function_return_hooked;
                data.function_return_hooked = true;
                function_name = data.function_name.clone();                
                
                match (data.callback)(client, bp) {
                    Ok(i) => i,
                    Err(e) => {
                        let _ = dbgeng::dlogln!(client, "Error in breakpoint callback: {e:?}");
                        DebugInstruction::NoChange
                    }
                }
            } else {
                DebugInstruction::NoChange
            };

        if need_to_hook {
            // set a bp on the return to read the 
            let stack = client.context_stack_frames(1).unwrap();
            let ro = stack[0].ReturnOffset;        
            let bp = client.add_breakpoint(BreakpointType::Code, None).unwrap();
            let _ = bp.set_offset(ro);
            let _ = bp.set_flags(BreakpointFlags::ENABLED);
            let _ = dbgeng::dlogln!(client, "*** Hook {} return address at 0x{:x}",  function_name, ro);

            inner.insert(bp.guid().unwrap(), CallbackBreakpointData {
                bp,
                function_name: function_name.clone(),
                function_return_hooked: true,
                callback: Box::new(move |client, _| { logger::monitored_func_end(client, function_name.clone()) })
            });
        }

        result
    }
}