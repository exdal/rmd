use std::{
    ffi::{CStr, CString, c_char, c_int},
    sync::{
        Mutex,
        atomic::{AtomicU32, Ordering},
    },
};

static RESULT: Mutex<Option<CString>> = Mutex::new(None);
static COUNT: AtomicU32 = AtomicU32::new(0);

fn give(text: String) -> *const c_char {
    let mut result = RESULT.lock().unwrap();
    result.insert(CString::new(text).unwrap()).as_ptr()
}

#[no_mangle]
pub unsafe extern "C" fn merge(argc: c_int, argv: *const *const c_char) -> *const c_char {
    let mut merged = String::new();
    for index in 0..argc as usize {
        merged.push_str(&CStr::from_ptr(*argv.add(index)).to_string_lossy());
    }

    give(merged)
}

#[no_mangle]
pub extern "C" fn count(_: c_int, _: *const *const c_char) -> *const c_char {
    give((COUNT.fetch_add(1, Ordering::Relaxed) + 1).to_string())
}

#[no_mangle]
pub extern "C" fn nothing(_: c_int, _: *const *const c_char) -> *const c_char {
    std::ptr::null()
}

#[no_mangle]
pub extern "C" fn cwd(_: c_int, _: *const *const c_char) -> *const c_char {
    give(std::env::current_dir().unwrap().display().to_string())
}

#[no_mangle]
pub extern "C" fn touch(_: c_int, _: *const *const c_char) -> *const c_char {
    std::fs::create_dir_all("made").unwrap();
    std::fs::write("made/out.txt", "").unwrap();
    give(String::new())
}
