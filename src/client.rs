// Axel '0vercl0k' Souchet - January 21 2024
//! This contains the main class, [`DebugClient`], which is used to interact
//! with Microsoft's Debug Engine library via the documented COM objects.
use std::collections::HashMap;
use std::ffi::{CString, OsStr};
use std::mem::MaybeUninit;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use windows::core::{IUnknown, Interface};
use windows::Win32::System::Diagnostics::Debug::Extensions::{
    IDebugControl3, IDebugDataSpaces4, IDebugRegisters, IDebugSymbols3, DEBUG_ADDSYNTHMOD_DEFAULT,
    DEBUG_EXECUTE_DEFAULT, DEBUG_OUTCTL_ALL_CLIENTS, DEBUG_OUTPUT_NORMAL, DEBUG_VALUE,
    DEBUG_VALUE_FLOAT128, DEBUG_VALUE_FLOAT32, DEBUG_VALUE_FLOAT64, DEBUG_VALUE_FLOAT80,
    DEBUG_VALUE_INT16, DEBUG_VALUE_INT32, DEBUG_VALUE_INT64, DEBUG_VALUE_INT8,
    DEBUG_VALUE_VECTOR128, DEBUG_VALUE_VECTOR64,
};
use windows::Win32::System::Diagnostics::Debug::IMAGE_NT_HEADERS32;
use windows::Win32::System::SystemInformation::IMAGE_FILE_MACHINE;
use windows::Win32::System::SystemServices::{
    IMAGE_DOS_HEADER, IMAGE_DOS_SIGNATURE, IMAGE_NT_SIGNATURE,
};

use crate::as_pcstr::AsPCSTR;
use crate::bits::Bits;

/// Extract [`u128`] off a [`DEBUG_VALUE`].
pub fn u128_from_debugvalue(v: DEBUG_VALUE) -> Result<u128> {
    let value = match v.Type {
        DEBUG_VALUE_FLOAT80 => {
            let f80 = unsafe { v.Anonymous.F80Bytes };
            let mut bytes = [0; 16];
            bytes[0..10].copy_from_slice(&f80);

            u128::from_le_bytes(bytes)
        }
        DEBUG_VALUE_VECTOR128 => u128::from_le_bytes(unsafe { v.Anonymous.VI8 }),
        DEBUG_VALUE_FLOAT128 => u128::from_le_bytes(unsafe { v.Anonymous.F128Bytes }),
        _ => {
            bail!("expected float128 values, but got Type={:#x}", v.Type);
        }
    };

    Ok(value)
}

/// Extract a [`u64`]/[`u32`]/[`u16`]/[`u8`]/[`f64`] off a [`DEBUG_VALUE`].
pub fn u64_from_debugvalue(v: DEBUG_VALUE) -> Result<u64> {
    let value = match v.Type {
        DEBUG_VALUE_INT64 => {
            let parts = unsafe { v.Anonymous.I64Parts32 };

            (u64::from(parts.HighPart) << 32) | u64::from(parts.LowPart)
        }
        DEBUG_VALUE_INT32 => unsafe { v.Anonymous.I32 }.into(),
        DEBUG_VALUE_INT16 => unsafe { v.Anonymous.I16 }.into(),
        DEBUG_VALUE_INT8 => unsafe { v.Anonymous.I8 }.into(),
        DEBUG_VALUE_VECTOR64 => {
            u64::from_le_bytes(unsafe { &v.Anonymous.VI8[0..8] }.try_into().unwrap())
        }
        DEBUG_VALUE_FLOAT64 => unsafe { v.Anonymous.F64 }.to_bits(),
        DEBUG_VALUE_FLOAT32 => f64::from(unsafe { v.Anonymous.F32 }).to_bits(),
        _ => {
            bail!("expected int/float values, but got Type={:#x}", v.Type);
        }
    };

    Ok(value)
}

/// Intel x86 segment descriptor.
#[derive(Default, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Seg {
    /// Is the segment present?
    pub present: bool,
    /// Segment selector.
    pub selector: u16,
    /// Base address.
    pub base: u64,
    /// Limit.
    pub limit: u32,
    /// Segment attributes.
    pub attr: u16,
}

impl Seg {
    /// Build a [`Seg`] from a `selector` and its raw value as read in the GDT.
    pub fn from_descriptor(selector: u64, value: u128) -> Self {
        let limit = (value.bits(0..=15) | (value.bits(48..=51) << 16)) as u32;
        let mut base = value.bits(16..=39) | (value.bits(56..=63) << 24);
        let present = value.bit(47) == 1;
        let attr = value.bits(40..=55) as u16;
        let selector = selector as u16;
        let non_system = value.bit(44);
        if non_system == 0 {
            base |= value.bits(64..=95) << 32;
        }

        Seg {
            present,
            selector,
            base: base as u64,
            limit,
            attr,
        }
    }
}

/// Macro to make it nicer to invoke [`DebugClient::logln`] /
/// [`DebugClient::log`] by avoiding to [`format!`] everytime the arguments.
#[macro_export]
macro_rules! dlogln {
    ($dbg:expr, $($arg:tt)*) => {{
        $dbg.logln(format!($($arg)*))
    }};
}

#[macro_export]
macro_rules! dlog {
    ($dbg:expr, $($arg:tt)*) => {{
        $dbg.log(format!($($arg)*))
    }};
}

#[derive(Clone)]
/// A debug client wraps a bunch of COM interfaces and provides higher level
/// features such as dumping registers, reading the GDT, reading virtual memory,
/// etc.
pub struct DebugClient {
    client: IDebugClient8,
    control: IDebugControl4,
    registers: IDebugRegisters,
    dataspaces: IDebugDataSpaces4,
    symbols: IDebugSymbols3,
    system: IDebugSystemObjects4,
}

impl DebugClient {
    pub fn new(client: &IUnknown) -> Result<Self> {
        let control = client.cast()?;
        let registers = client.cast()?;
        let dataspaces = client.cast()?;
        let symbols = client.cast()?;
        let system = client.cast()?;
        let client = client.cast()?;        
        
        Ok(Self {
            client,
            control,
            registers,
            dataspaces,
            symbols,
            system,
        })
    }

    /// Create a new instance of the debug client interface.
    pub fn create() -> Result<Self> {
        unsafe {
            DebugCreate::<IUnknown>()
                .map(|c| Self::new(&c).unwrap())
                .map_err(|e| e.into())
        }
    }

    /// Output a message `s`.
    fn output<Str>(&self, mask: u32, s: Str) -> Result<()>
    where
        Str: Into<Vec<u8>>,
    {
        let cstr = CString::new(s.into()).context("failed to convert output string")?;
        unsafe { self.control.Output(mask, cstr.as_pcstr()) }.context("Output failed")
    }

    /// Log a message in the debugging window.
    #[allow(dead_code)]
    pub fn log<Str>(&self, args: Str) -> Result<()>
    where
        Str: Into<Vec<u8>>,
    {
        self.output(DEBUG_OUTPUT_NORMAL, args)
    }

    /// Log a message followed by a new line in the debugging window.
    pub fn logln<Str>(&self, args: Str) -> Result<()>
    where
        Str: Into<Vec<u8>>,
    {
        self.output(DEBUG_OUTPUT_NORMAL, args)?;
        self.output(DEBUG_OUTPUT_NORMAL, "\n")
    }

    /// Execute a debugger command.
    pub fn exec<Str>(&self, cmd: Str) -> Result<()>
    where
        Str: Into<Vec<u8>>,
    {
        let cstr = CString::new(cmd.into())?;
        unsafe {
            self.control.Execute(
                DEBUG_OUTCTL_ALL_CLIENTS,
                cstr.as_pcstr(),
                DEBUG_EXECUTE_DEFAULT,
            )
        }
        .with_context(|| format!("Execute({:?}) failed", cstr))
    }

    /// Get up to N stack frames in the current debugger context.
    pub fn context_stack_frames(&self, n: usize) -> Result<Vec<DEBUG_STACK_FRAME>> {
        let mut stack = vec![DEBUG_STACK_FRAME::default(); n];
        let mut frames_filled = 0;
        unsafe {
            self.control.GetContextStackTrace(
                None,
                0,
                Some(&mut stack),
                None,
                0,
                0,
                Some(&mut frames_filled),
            )
        }
        .context("GetContextStackTrace failed")?;

        stack.resize(frames_filled.try_into()?, DEBUG_STACK_FRAME::default());

        Ok(stack)
    }

    /// Setup an object to receive debugger event callbacks.
    pub fn set_event_callbacks<E: EventCallbacks + 'static>(&self, e: E) -> Result<()> {
        let callbacks = Box::new(e);
        let callbacks: IUnknown = DbgEventCallbacks::new(self.clone(), callbacks).into();

        unsafe {
            self.client
                .SetEventCallbacks(&callbacks.cast::<IDebugEventCallbacks>()?)
        }
        .context("SetEventCallbacks failed")
    }

    /// Create a new breakpoint.
    pub fn add_breakpoint(
        &self,
        ty: BreakpointType,
        desired_id: Option<u32>,
    ) -> Result<DebugBreakpoint> {
        let bp = unsafe {
            self.control.AddBreakpoint(
                match ty {
                    BreakpointType::Code => DEBUG_BREAKPOINT_CODE,
                    BreakpointType::Data => DEBUG_BREAKPOINT_DATA,
                },
                desired_id.unwrap_or(DEBUG_ANY_ID),
            )
        }
        .context("AddBreakpoint failed")?;
        DebugBreakpoint::new(bp)
    }

    /// Remove a previously created breakpoint.
    pub fn remove_breakpoint(
        &self,
        bp: DebugBreakpoint
    ) -> Result<()> {        
        unsafe { 
            let i: IUnknown = bp.0.into();
            self.control.RemoveBreakpoint(&i.cast::<IDebugBreakpoint>().unwrap()) 
            .context("RemoveBreakpoint failed")?;
        };
        Ok(())
    }
    
    /// Get the register indices from names.
    pub fn reg_indices(&self, names: &[&str]) -> Result<Vec<u32>> {
        let mut indices = Vec::with_capacity(names.len());
        for name in names {
            let indice = unsafe {
                self.registers
                    .GetIndexByName(CString::new(*name)?.as_pcstr())
            }
            .with_context(|| format!("GetIndexByName failed for {name}"))?;

            indices.push(indice);
        }

        Ok(indices)
    }

    /// Get the value of multiple registers.
    pub fn reg_values(&self, indices: &[u32]) -> Result<Vec<DEBUG_VALUE>> {
        let mut values = vec![DEBUG_VALUE::default(); indices.len()];
        unsafe {
            self.registers.GetValues(
                indices.len().try_into()?,
                Some(indices.as_ptr()),
                0,
                values.as_mut_ptr(),
            )
        }
        .with_context(|| format!("GetValues failed for {indices:?}"))?;

        Ok(values)
    }

    /// Get [`u128`] values for the registers identified by their names.
    pub fn regs128(&self, names: &[&str]) -> Result<Vec<u128>> {
        let indices = self.reg_indices(names)?;
        let values = self.reg_values(&indices)?;

        values.into_iter().map(u128_from_debugvalue).collect()
    }

    /// Get [`u128`] values for the registers identified by their names but
    /// returned in a dictionary with their names.
    pub fn regs128_dict<'a>(&self, names: &[&'a str]) -> Result<HashMap<&'a str, u128>> {
        let values = self.regs128(names)?;

        Ok(HashMap::from_iter(
            names.iter().zip(values).map(|(k, v)| (*k, v)),
        ))
    }

    /// Get the values of a set of registers identified by their names.
    pub fn regs64(&self, names: &[&str]) -> Result<Vec<u64>> {
        let indices = self.reg_indices(names)?;
        let values = self.reg_values(&indices)?;

        values.into_iter().map(u64_from_debugvalue).collect()
    }

    /// Get the values of a set of registers identified by their names and store
    /// both their names / values in a dictionary.
    pub fn regs64_dict<'a>(&self, names: &[&'a str]) -> Result<HashMap<&'a str, u64>> {
        let values = self.regs64(names)?;

        Ok(HashMap::from_iter(
            names.iter().zip(values).map(|(k, v)| (*k, v)),
        ))
    }

    /// Get the value of a register identified by its name.
    pub fn reg64(&self, name: &str) -> Result<u64> {
        let v = self.regs64(&[name])?;

        Ok(v[0])
    }

    /// Set the value of a register identified by uts name
    pub fn set_reg64(&self, name: &str, value: u64) -> Result<()> {
        let indices = self.reg_indices(&[name])?; 
        unsafe {                         
            let mut debug_value = DEBUG_VALUE::default();          
            debug_value.Anonymous.I64Parts32.HighPart = (value >> 32) as u32;
            debug_value.Anonymous.I64Parts32.LowPart =  value as u32;
            debug_value.Type = DEBUG_VALUE_INT64;           
            self.registers.SetValue(indices[0], &debug_value)
                .with_context(|| format!("SetValue failed for {name}"))?;
        }

        Ok(())
    }

    /// Get the value of a specific MSR.
    pub fn msr(&self, msr: u32) -> Result<u64> {
        unsafe { self.dataspaces.ReadMsr(msr) }.context("ReadMsr failed")
    }

    /// Read a segment descriptor off the GDT.
    pub fn gdt_entry(&self, gdt_base: u64, gdt_limit: u16, selector: u64) -> Result<Seg> {
        // Let's first get the index out of the selector; here's what the selector looks
        // like (Figure 3-6. Segment Selector):
        //
        // 15                                                 3    2        0
        // +--------------------------------------------------+----+--------+
        // |          Index                                   | TI |   RPL  |
        // +--------------------------------------------------+----+--------+
        //
        // TI = Table Indicator: 0 = GDT, 1 = LDT
        //

        // The function will read the descriptor off the GDT, so let's make sure the
        // table indicator matches that.
        let ti = selector.bit(2);
        if ti != 0 {
            bail!("expected a GDT table indicator when reading segment descriptor");
        }

        // Extract the index so that we can calculate the address of the GDT entry.
        let index = selector.bits(3..=15);
        // 3.5.1 Segment Descriptor Tables
        // "As with segments, the limit value is added to the base address to get the
        // address of the last valid byte. A limit value of 0 results in exactly one
        // valid byte. Because segment descriptors are always 8 bytes long, the GDT
        // limit should always be one less than an integral multiple of eight (that is,
        // 8N – 1)"
        let gdt_limit = gdt_limit as u64;
        assert!((gdt_limit + 1) % 8 == 0);
        let max_index = (gdt_limit + 1) / 8;
        if index >= max_index {
            bail!("the selector {selector:#x} has an index ({index:#x}) larger than the maximum allowed ({max_index:#})");
        }

        // Most GDT entries are 8 bytes long but some are 16, so accounting for that.
        //
        // 3.5 SYSTEM DESCRIPTOR TYPES
        // "When the S (descriptor type) flag in a segment descriptor is clear, the
        // descriptor type is a system descriptor." "Note that system
        // descriptors in IA-32e mode are 16 bytes instead of 8 bytes."
        let mut descriptor = [0; 16];
        // 3.4.2 Segment Selectors
        // "The processor multiplies the index value by 8 (the number of bytes in a
        // segment descriptor).."
        let entry_addr = gdt_base + (index * 8u64);

        // Read the entry.
        self.read_virtual_exact(entry_addr, &mut descriptor)?;

        // Build the descriptor.
        Ok(Seg::from_descriptor(
            selector,
            u128::from_le_bytes(descriptor),
        ))
    }

    /// Read virtual memory as a field.
    pub fn read_virtual_struct<
        T: zerocopy::AsBytes + zerocopy::FromBytes + zerocopy::FromZeroes,
    >(
        &self,
        vaddr: u64,
    ) -> Result<T> {
        let mut buffer = T::new_zeroed();
        self.read_virtual_exact(vaddr, buffer.as_bytes_mut())?;

        Ok(buffer)
    }

    /// Read an exact amount of virtual memory.
    pub fn read_virtual_exact(&self, vaddr: u64, buf: &mut [u8]) -> Result<()> {
        let amount_read = self.read_virtual(vaddr, buf)?;
        if amount_read != buf.len() {
            bail!(
                "expected to read_virtual {:#x} bytes, but read {:#x}",
                buf.len(),
                amount_read
            );
        }

        Ok(())
    }

    /// Read virtual memory.
    pub fn read_virtual(&self, vaddr: u64, buf: &mut [u8]) -> Result<usize> {
        let mut amount_read = 0;
        unsafe {
            self.dataspaces.ReadVirtual(
                vaddr,
                buf.as_mut_ptr().cast(),
                buf.len().try_into()?,
                Some(&mut amount_read),
            )
        }
        .context("ReadVirtual failed")?;

        Ok(usize::try_from(amount_read)?)
    }

    /// Look up a module by name.
    pub fn get_sym_module(&self, name: &str) -> Result<SymbolModule> {
        let name_cstr = CString::new(name).context("failed to wrap module string")?;
        let mut base = 0u64;
        unsafe {
            self.symbols
                .GetModuleByModuleName(name_cstr.as_pcstr(), 0, None, Some(&mut base))
        }
        .context("GetModuleByModuleName failed")?;

        Ok(SymbolModule::new(self.symbols.clone(), base))
    }

    /// Get the debuggee type.
    pub fn debuggee_type(&self) -> Result<(u32, u32)> {
        let mut class = 0;
        let mut qualifier = 0;
        unsafe { self.control.GetDebuggeeType(&mut class, &mut qualifier) }?;

        Ok((class, qualifier))
    }

    /// Get the processor type of the target.
    pub fn processor_type(&self) -> Result<IMAGE_FILE_MACHINE> {
        let proc_type = unsafe { self.control.GetActualProcessorType() }
            .context("GetActualProcessorType failed")?;

        Ok(IMAGE_FILE_MACHINE(proc_type.try_into()?))
    }

    /// Get the number of processors in the target.
    pub fn processor_number(&self) -> Result<u32> {
        unsafe { self.control.GetNumberProcessors() }.context("GetNumberProcessors failed")
    }

    /// Get an address for a named symbol.
    pub fn get_address_by_name<Str>(&self, symbol: Str) -> Result<u64>
    where
        Str: Into<Vec<u8>>,
    {
        let symbol_cstr = CString::new(symbol.into())?;

        unsafe { self.symbols.GetOffsetByName(symbol_cstr.as_pcstr()) }
            .context("GetOffsetByName failed")
    }

    /// Read a NULL terminated string at `addr`.
    pub fn read_cstring_virtual(&self, addr: u64) -> Result<String> {
        let maxbytes = 100;
        let mut buffer = vec![0; maxbytes];
        let mut length = 0;
        unsafe {
            self.dataspaces.ReadMultiByteStringVirtual(
                addr,
                maxbytes as u32,
                Some(buffer.as_mut()),
                Some(&mut length),
            )
        }?;

        if length == 0 {
            bail!("length is zero")
        }

        let length = length as usize;
        buffer.resize(length - 1, 0);

        Ok(String::from_utf8_lossy(&buffer).into_owned())
    }

    pub fn read_wstring_virtual(&self, addr: u64) -> Result<String> {
        let maxbytes = 100;
        let mut buffer = vec![0; maxbytes];
        let mut length = 0;
        unsafe {
            self.dataspaces.ReadUnicodeStringVirtual(
                addr,
                maxbytes as u32,
                65001, // CP_UTF8
                Some(&mut buffer),
                Some(&mut length),
            )
        }
        .context("ReadUnicodeStringVirtual failed")?;

        if length == 0 {
            bail!("length is zero")
        }

        let length = length as usize;
        buffer.resize(length - 1, 0);

        Ok(String::from_utf8_lossy(&buffer).into_owned())
    }

    pub fn get_current_process_id(&self) -> Result<u32> {
        let process_id = unsafe {
            self.system.GetCurrentProcessSystemId()
        }
        .context("GetCurrentProcessId failed")?;
        Ok(process_id)
    }

    pub fn get_current_thread_id(&self) -> Result<u32> {
        let thread_id = unsafe {
            self.system.GetCurrentThreadSystemId()
        }
        .context("GetCurrentThreadId failed")?;
        Ok(thread_id)
    }
}
