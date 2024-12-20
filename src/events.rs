use std::panic::AssertUnwindSafe;

use windows::core::{implement, HRESULT};
use windows::Win32::System::Diagnostics::Debug::Extensions::{
    IDebugBreakpoint, IDebugEventCallbacks, IDebugEventCallbacks_Impl, 
    DEBUG_EVENT_BREAKPOINT, DEBUG_EVENT_CHANGE_ENGINE_STATE, DEBUG_EVENT_EXCEPTION,
    DEBUG_STATUS_BREAK, DEBUG_STATUS_GO, DEBUG_STATUS_GO_HANDLED, DEBUG_STATUS_GO_NOT_HANDLED, 
    DEBUG_STATUS_IGNORE_EVENT, DEBUG_STATUS_NO_CHANGE, DEBUG_STATUS_RESTART_REQUESTED, 
    DEBUG_STATUS_STEP_BRANCH, DEBUG_STATUS_STEP_INTO, DEBUG_STATUS_STEP_OVER
};
use windows::Win32::System::Diagnostics::Debug::EXCEPTION_RECORD64;

use crate::breakpoint::DebugBreakpoint;
use crate::exception::ExceptionInfo;
use crate::client::DebugClient;
use crate::dlogln;

/// An instruction for the debugger to follow.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugInstruction {
    /// Suspend the target.
    Break,
    /// Continue execution for a single instruction (stepping into call
    /// instructions).
    StepInto,
    /// Continue execution until the next branch instruction.
    StepBranch,
    /// Continue execution for a single instruction, stepping over call
    /// instructions.
    StepOver,
    /// Continue execution and set the event as unhandled.
    GoNotHandled,
    /// Continue execution and flag the event as handled.
    GoHandled,
    /// Continue execution.
    Go,
    /// Ignore the event.
    IgnoreEvent,
    /// Restart the target.
    Restart,
    /// No instruction; return if your event handler is uninterested in the
    /// event.
    #[default]
    NoChange,
}

impl DebugInstruction {
    fn as_status(&self) -> u32 {
        match self {
            DebugInstruction::Break => DEBUG_STATUS_BREAK,
            DebugInstruction::StepInto => DEBUG_STATUS_STEP_INTO,
            DebugInstruction::StepBranch => DEBUG_STATUS_STEP_BRANCH,
            DebugInstruction::StepOver => DEBUG_STATUS_STEP_OVER,
            DebugInstruction::GoNotHandled => DEBUG_STATUS_GO_NOT_HANDLED,
            DebugInstruction::GoHandled => DEBUG_STATUS_GO_HANDLED,
            DebugInstruction::Go => DEBUG_STATUS_GO,
            DebugInstruction::IgnoreEvent => DEBUG_STATUS_IGNORE_EVENT,
            DebugInstruction::Restart => DEBUG_STATUS_RESTART_REQUESTED,
            DebugInstruction::NoChange => DEBUG_STATUS_NO_CHANGE,
        }
    }
}

pub trait EventCallbacks {
    fn breakpoint(&self, _client: &DebugClient, _bp: &DebugBreakpoint) -> DebugInstruction;
    fn exception(&self, _client: &DebugClient, _ei: &ExceptionInfo) -> DebugInstruction;
    fn change_engine_state(&self, _client: &DebugClient, _flags: u32, _argument: u64);
}

#[implement(IDebugEventCallbacks)]
pub(crate) struct DbgEventCallbacks {
    client: DebugClient,
    callbacks: Box<dyn EventCallbacks>,
}

impl DbgEventCallbacks {
    pub(crate) fn new(client: DebugClient, callbacks: Box<dyn EventCallbacks + 'static>) -> Self {
        Self { client, callbacks }
    }
}

impl IDebugEventCallbacks_Impl for DbgEventCallbacks {
    fn GetInterestMask(&self) -> windows::core::Result<u32> {
        Ok(
            DEBUG_EVENT_BREAKPOINT | 
            DEBUG_EVENT_EXCEPTION | 
            DEBUG_EVENT_CHANGE_ENGINE_STATE
        )
    }

    fn Breakpoint(
        &self,
        bp: ::core::option::Option<&IDebugBreakpoint>,
    ) -> windows::core::Result<()> {
        let bp = bp
            .expect("breakpoint callback called with NULL breakpoint")
            .to_owned();

        // N.B: The breakpoint must be represented as "borrowed" because it could be
        // invalid after this callback returns.
        let res = std::panic::catch_unwind(AssertUnwindSafe(|| {
            self.callbacks
                .breakpoint(&self.client, &DebugBreakpoint::new(bp).unwrap())
        }));

        let res = match res {
            Ok(i) => i,
            Err(panic) => {
                // If the callback panics, we'll just ignore it and continue.
                let _ = dlogln!(self.client, "panic in breakpoint callback: {:?}", panic);
                DebugInstruction::GoNotHandled
            }
        };

        // N.B: This is pretty lame; the API is declared to return a HRESULT, but it
        // does not actually return a HRESULT. We'll need to shim our return
        // value into a HRESULT-looking thing. Ok(_) maps to 0, and Err(e) maps
        // to e.code(). So we'll always return an "error".
        let res = HRESULT(res.as_status() as i32);
        Err(res.into())
    }

    fn Exception(
        &self,
        exception: *const EXCEPTION_RECORD64,
        firstchance: u32,
    ) -> windows::core::Result<()> {     
        let exception_info = ExceptionInfo {
            record: unsafe { exception.read().into() },
            first_chance: firstchance
        };
       
        let res = std::panic::catch_unwind(AssertUnwindSafe(|| {
            self.callbacks
                .exception(&self.client, &exception_info)
        }));

        let res = match res {
            Ok(i) => i,
            Err(panic) => {
                // If the callback panics, we'll just ignore it and continue.
                let _ = dlogln!(self.client, "panic in exception callback: {:?}", panic);
                DebugInstruction::GoNotHandled
            }
        };

        // N.B: This is pretty lame; the API is declared to return a HRESULT, but it
        // does not actually return a HRESULT. We'll need to shim our return
        // value into a HRESULT-looking thing. Ok(_) maps to 0, and Err(e) maps
        // to e.code(). So we'll always return an "error".
        let res = HRESULT(res.as_status() as i32);
        Err(res.into())
    }

    fn CreateThread(
        &self,
        _handle: u64,
        _dataoffset: u64,
        _startoffset: u64,
    ) -> windows::core::Result<()> {
        let _ = dlogln!(self.client, "Event: CreateThread");
        Ok(())
    }

    fn ExitThread(&self, _exitcode: u32) -> windows::core::Result<()> {
        let _ = dlogln!(self.client, "Event: ExitThreat");
        Ok(())
    }

    fn CreateProcessA(
        &self,
        _imagefilehandle: u64,
        _handle: u64,
        _baseoffset: u64,
        _modulesize: u32,
        _modulename: &windows::core::PCSTR,
        _imagename: &windows::core::PCSTR,
        _checksum: u32,
        _timedatestamp: u32,
        _initialthreadhandle: u64,
        _threaddataoffset: u64,
        _startoffset: u64,
    ) -> windows::core::Result<()> {
        let _ = dlogln!(self.client, "Event: CreateProcessA");
        Ok(())
    }

    fn ExitProcess(&self, _exitcode: u32) -> windows::core::Result<()> {
        let _ = dlogln!(self.client, "Event: ExitProcess");
        Ok(())
    }

    fn LoadModule(
        &self,
        _imagefilehandle: u64,
        _baseoffset: u64,
        _modulesize: u32,
        _modulename: &windows::core::PCSTR,
        _imagename: &windows::core::PCSTR,
        _checksum: u32,
        _timedatestamp: u32,
    ) -> windows::core::Result<()> {
        let _ = dlogln!(self.client, "Event: LoadModule");
        Ok(())
    }

    fn UnloadModule(
        &self,
        _imagebasename: &windows::core::PCSTR,
        _baseoffset: u64,
    ) -> windows::core::Result<()> {
        let _ = dlogln!(self.client, "Event: UnloadModule");
        Ok(())
    }

    fn SystemError(&self, _error: u32, _level: u32) -> windows::core::Result<()> {
        let _ = dlogln!(self.client, "Event: SystemError");
        Ok(())
    }

    fn SessionStatus(&self, _status: u32) -> windows::core::Result<()> {
        let _ = dlogln!(self.client, "Event: SessionStatus");
        Ok(())
    }

    fn ChangeDebuggeeState(&self, _flags: u32, _argument: u64) -> windows::core::Result<()> {
        let _ = dlogln!(self.client, "Event: ChangeDebuggeeState");
        Ok(())
    }

    fn ChangeEngineState(&self, flags: u32, argument: u64) -> windows::core::Result<()> {
         let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
            self.callbacks
                .change_engine_state(&self.client, flags, argument)
        }));
        Ok(())
    }

    fn ChangeSymbolState(&self, _flags: u32, _argument: u64) -> windows::core::Result<()> {
        let _ = dlogln!(self.client, "Event: ChangeSymbolState");
        Ok(())
    }
}
