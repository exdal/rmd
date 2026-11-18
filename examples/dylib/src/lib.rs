use std::{
    ffi::{CStr, CString, c_char, c_int},
    hash::{DefaultHasher, Hash, Hasher},
    sync::{
        Mutex,
        atomic::{AtomicU32, Ordering},
    },
};

static RESULT: Mutex<Option<CString>> = Mutex::new(None);
static CALLS: AtomicU32 = AtomicU32::new(0);

fn args<'a>(argc: c_int, argv: *const *const c_char) -> Vec<&'a str> {
    (0..argc.max(0) as usize)
        .map(|index| unsafe { CStr::from_ptr(*argv.add(index)) }.to_str().unwrap_or_default())
        .collect::<Vec<_>>()
}

fn give(text: String) -> *const c_char {
    let text = CString::new(text).unwrap_or_default();
    let mut result = RESULT.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    result.insert(text).as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn greet(argc: c_int, argv: *const *const c_char) -> *const c_char {
    let name = args(argc, argv).first().copied().unwrap_or("stranger");

    give(format!("Hello, {name}!"))
}

#[unsafe(no_mangle)]
pub extern "C" fn tint(argc: c_int, argv: *const *const c_char) -> *const c_char {
    let mut hasher = DefaultHasher::new();
    args(argc, argv).hash(&mut hasher);
    let [red, green, blue, ..] = hasher.finish().to_le_bytes();
    let pastel = |channel: u8| 160 + channel % 96;

    give(format!("#{:02x}{:02x}{:02x}", pastel(red), pastel(green), pastel(blue)))
}

#[unsafe(no_mangle)]
pub extern "C" fn calls(_argc: c_int, _argv: *const *const c_char) -> *const c_char {
    give((CALLS.fetch_add(1, Ordering::Relaxed) + 1).to_string())
}
