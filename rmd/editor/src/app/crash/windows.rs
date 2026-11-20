use std::{
    fs, io,
    os::windows::io::AsRawHandle,
    path::PathBuf,
    ptr,
    sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering},
    thread,
    time::SystemTime,
};

use windows_sys::Win32::System::{
    Diagnostics::Debug::{
        EXCEPTION_CONTINUE_SEARCH, EXCEPTION_EXECUTE_HANDLER, EXCEPTION_POINTERS, MINIDUMP_EXCEPTION_INFORMATION,
        MiniDumpWithIndirectlyReferencedMemory, MiniDumpWithThreadInfo, MiniDumpWriteDump, SetUnhandledExceptionFilter,
    },
    LibraryLoader::{
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW,
        GetModuleHandleExW,
    },
    Threading::{GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId},
};

use super::{
    super::dialog::show_error, CrashInfo, crash_message, create_unique, handle_main_thread_crash, last_report, record,
    recovery_summary, stamp,
};
use crate::{app::logging, session::Session, settings::paths::crash_dir};

static SESSION: AtomicPtr<Session> = AtomicPtr::new(ptr::null_mut());
static MAIN_THREAD: AtomicU32 = AtomicU32::new(0);
static IS_HANDLING: AtomicBool = AtomicBool::new(false);

const HELPER_STACK: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy)]
struct Fault {
    pointers: usize,
    thread_id: u32,
}

pub(super) fn install() {
    MAIN_THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::Relaxed);
    unsafe { SetUnhandledExceptionFilter(Some(filter)) };
}

pub(in crate::app) fn watch_session(session: *const Session) {
    SESSION.store(session.cast_mut(), Ordering::Release);
}

pub(in crate::app) fn unwatch_session() {
    SESSION.store(ptr::null_mut(), Ordering::Release);
}

pub(super) fn dump_for_panic() -> Option<PathBuf> {
    let thread_id = unsafe { GetCurrentThreadId() };
    let helper = thread::Builder::new()
        .name("crash dump".to_owned())
        .spawn(move || write_dump_now(thread_id, None));

    helper.ok()?.join().ok().flatten()
}

unsafe extern "system" fn filter(pointers: *const EXCEPTION_POINTERS) -> i32 {
    if IS_HANDLING.swap(true, Ordering::AcqRel) {
        return EXCEPTION_CONTINUE_SEARCH;
    }

    let fault = Fault {
        pointers: pointers as usize,
        thread_id: unsafe { GetCurrentThreadId() },
    };
    let helper = thread::Builder::new()
        .name("crash handler".to_owned())
        .stack_size(HELPER_STACK)
        .spawn(move || handle_fault(fault));
    match helper {
        Ok(helper) => {
            let _ = helper.join();
        },
        Err(_) => handle_fault(fault),
    }

    EXCEPTION_EXECUTE_HANDLER
}

fn handle_fault(fault: Fault) {
    let pointers = fault.pointers as *mut EXCEPTION_POINTERS;
    let (code, address) = unsafe {
        let record = &*(*pointers).ExceptionRecord;
        (record.ExceptionCode as u32, record.ExceptionAddress as usize)
    };
    let message = format!("{} (0x{code:08X})", exception_name(code));
    let location = describe_address(address);
    logging::record_crash(&format!("{message} at {location}"));

    let dump = write_dump_now(fault.thread_id, Some(pointers));
    let is_main_thread = fault.thread_id == MAIN_THREAD.load(Ordering::Relaxed);
    let thread = if is_main_thread {
        "main".to_owned()
    } else {
        format!("native thread {}", fault.thread_id)
    };
    let mut info = CrashInfo::new(thread, message, location, "unavailable for native crashes".to_owned());
    info.dump = dump;
    record(&info);

    let session = SESSION.load(Ordering::Acquire);
    let message = if !is_main_thread {
        crash_message(last_report().as_deref(), "Unsaved changes could not be recovered.")
    } else if session.is_null() {
        crash_message(last_report().as_deref(), &recovery_summary(&[], &[]))
    } else {
        // the main thread is parked in the filter, so nothing mutates the session while we read it
        handle_main_thread_crash(unsafe { &*session })
    };

    show_error("rmd crashed", &message);
}

fn write_dump_now(thread_id: u32, pointers: Option<*mut EXCEPTION_POINTERS>) -> Option<PathBuf> {
    match write_dump(thread_id, pointers) {
        Ok(path) => Some(path),
        Err(error) => {
            eprintln!("could not write the crash dump: {error}");

            None
        },
    }
}

fn write_dump(thread_id: u32, pointers: Option<*mut EXCEPTION_POINTERS>) -> io::Result<PathBuf> {
    let dir = crash_dir()?;
    let (path, file) = create_unique(&dir, "crash-", &stamp(SystemTime::now())?, "dmp")?;
    let information = pointers.map(|pointers| MINIDUMP_EXCEPTION_INFORMATION {
        ThreadId: thread_id,
        ExceptionPointers: pointers,
        ClientPointers: 0,
    });
    let is_written = unsafe {
        MiniDumpWriteDump(
            GetCurrentProcess(),
            GetCurrentProcessId(),
            file.as_raw_handle(),
            MiniDumpWithThreadInfo | MiniDumpWithIndirectlyReferencedMemory,
            information.as_ref().map_or(ptr::null(), ptr::from_ref),
            ptr::null(),
            ptr::null(),
        )
    } != 0;
    if !is_written {
        let error = io::Error::last_os_error();
        drop(file);
        let _ = fs::remove_file(&path);

        return Err(error);
    }

    Ok(path)
}

fn describe_address(address: usize) -> String {
    let mut module = ptr::null_mut();
    let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
    if unsafe { GetModuleHandleExW(flags, address as *const u16, &mut module) } == 0 {
        return format!("{address:#x}");
    }

    let mut name = [0u16; 260];
    let len = unsafe { GetModuleFileNameW(module, name.as_mut_ptr(), name.len() as u32) } as usize;
    let path = String::from_utf16_lossy(&name[..len]);
    let file = path.rsplit('\\').next().unwrap_or(&path);

    format!("{file}+{:#x}", address - module as usize)
}

fn exception_name(code: u32) -> &'static str {
    match code {
        0xC000_0005 => "access violation",
        0xC000_00FD => "stack overflow",
        0xC000_0094 => "integer divide by zero",
        0xC000_001D => "illegal instruction",
        0xC000_0409 => "stack buffer overrun",
        0xC000_0006 => "in-page error",
        0x8000_0003 => "breakpoint",
        _ => "native exception",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exception_codes_have_readable_names() {
        assert_eq!(exception_name(0xC000_0005), "access violation");
        assert_eq!(exception_name(0x1234), "native exception");
    }

    #[test]
    fn an_address_inside_the_executable_names_it() {
        let description = describe_address(exception_name as *const () as usize);

        assert!(description.contains(".exe+0x"), "{description}");
    }
}
