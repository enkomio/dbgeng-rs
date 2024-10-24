use std::convert::Into;

use windows::Win32::{Foundation::NTSTATUS, System::Diagnostics::Debug::EXCEPTION_RECORD64};

pub struct ExceptionRecord {
    pub exception_code: NTSTATUS,
    pub exception_flag: u32,
    pub exception_record: u64,
    pub exception_address: u64,
    pub number_parameters: u32,
    pub exception_information: [u64; 15]
}

pub struct ExceptionInfo {
    pub record: ExceptionRecord,
    pub first_chance: u32
}

impl Into<ExceptionRecord> for EXCEPTION_RECORD64 {
    fn into(self) -> ExceptionRecord {
        ExceptionRecord { 
            exception_code: self.ExceptionCode,
            exception_flag: self.ExceptionFlags,
            exception_record: self.ExceptionRecord,
            exception_address: self.ExceptionAddress,
            number_parameters: self.NumberParameters,
            exception_information: self.ExceptionInformation
        }
    }
}