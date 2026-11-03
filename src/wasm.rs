//! Minimal C-ABI surface so the engine can run in a browser (WebAssembly)
//! without any binding generator. One global session; strings cross the
//! boundary as (pointer, length) into wasm memory.
//!
//! JS calls: qk_init() once, then per word qk_push(ptr, len, times_ptr, n)
//! which returns a pointer to a JSON result whose length is qk_out_len().

use std::cell::RefCell;

use crate::{Config, Engine, Stream};

thread_local! {
    static ENGINE: &'static Engine = Box::leak(Box::new(Engine::new(Config::default())));
    static STREAM: RefCell<Option<Stream<'static>>> = const { RefCell::new(None) };
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn set_out(s: String) -> *const u8 {
    OUT.with(|o| {
        let mut o = o.borrow_mut();
        *o = s.into_bytes();
        o.as_ptr()
    })
}

#[no_mangle]
pub extern "C" fn qk_alloc(n: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(n.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

#[no_mangle]
pub extern "C" fn qk_free(p: *mut u8, n: usize) {
    unsafe { drop(Vec::from_raw_parts(p, 0, n.max(1))) }
}

#[no_mangle]
pub extern "C" fn qk_out_len() -> usize {
    OUT.with(|o| o.borrow().len())
}

/// Load the dictionary and start a fresh session.
#[no_mangle]
pub extern "C" fn qk_init() {
    qk_reset();
}

#[no_mangle]
pub extern "C" fn qk_reset() {
    let e = ENGINE.with(|e| *e);
    STREAM.with(|s| *s.borrow_mut() = Some(Stream::new(e)));
}

/// Commit one typed word (no spaces). `times` are key-down times in ms, one
/// per byte of the word, or n = 0 if unknown. Returns JSON:
/// {"edits":[{"i":3,"old":"wamt","new":"want","rev":false,"p":0.99}]}
#[no_mangle]
pub extern "C" fn qk_push(ptr: *const u8, len: usize, times: *const f32, n: usize) -> *const u8 {
    let word = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len)) }.to_string();
    let times = (n == len && n > 0).then(|| unsafe { std::slice::from_raw_parts(times, n) }.to_vec());
    let edits = STREAM.with(|s| s.borrow_mut().as_mut().map(|s| s.push(&word, times)).unwrap_or_default());
    let items: Vec<String> = edits
        .iter()
        .map(|e| {
            format!(
                "{{\"i\":{},\"old\":{},\"new\":{},\"rev\":{},\"p\":{:.3}}}",
                e.index,
                json_str(&e.old),
                json_str(&e.new),
                e.revision,
                e.confidence
            )
        })
        .collect();
    set_out(format!("{{\"edits\":[{}]}}", items.join(",")))
}

/// The user undid the correction of word `index` (e.g. Backspace right after it).
#[no_mangle]
pub extern "C" fn qk_reject(index: usize) -> *const u8 {
    let e = STREAM.with(|s| s.borrow_mut().as_mut().and_then(|s| s.reject(index)));
    set_out(match e {
        Some(e) => format!("{{\"i\":{},\"new\":{}}}", e.index, json_str(&e.new)),
        None => "null".into(),
    })
}
