use runtime_exec::run_wasm;
use std::time::{Duration, Instant};

const WASM_MOD: &[u8] = &[
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7e, 0x03,
    0x02, 0x01, 0x00, 0x07, 0x0a, 0x01, 0x06, 0x61, 0x6e, 0x73, 0x77, 0x65, 0x72, 0x00, 0x00, 0x0a,
    0x06, 0x01, 0x04, 0x00, 0x42, 0x2a, 0x0b,
];

fn main() {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mod_bytes = bytes::Bytes::from_static(WASM_MOD);
    match run_wasm(&mod_bytes, deadline) {
        Ok(tok) => {
            assert_eq!(tok.as_str(), "42", "wasm3 must return 42");
            println!("wasm3 ok: {}", tok.as_str());
        }
        Err(e) => {
            eprintln!("wasm3 fail: {e}");
            std::process::exit(1);
        }
    }
    let garbage = b"\x00asm\x01\x00\x00\x00\x99\x99";
    assert!(run_wasm(&bytes::Bytes::copy_from_slice(garbage), deadline).is_err(), "garbage must fail");
    println!("garbage rejected ok");
}
