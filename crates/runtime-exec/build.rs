use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src = manifest
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .join("third_party/wasm3/source");
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));

    let objects = [
        "m3_api_libc.c",
        "m3_bind.c",
        "m3_code.c",
        "m3_compile.c",
        "m3_core.c",
        "m3_emit.c",
        "m3_env.c",
        "m3_exec.c",
        "m3_function.c",
        "m3_info.c",
        "m3_module.c",
        "m3_optimize.c",
        "m3_parse.c",
    ];

    let opt = env::var("OPT_LEVEL").unwrap_or_else(|_| "2".into());
    let cflags = [
        format!("-O{opt}"),
        "-std=c99".into(),
        "-fno-strict-aliasing".into(),
        "-fno-plt".into(),
        "-fvisibility=hidden".into(),
        "-Wno-int-to-pointer-cast".into(),
        "-Wno-sign-compare".into(),
        "-Wno-unused-parameter".into(),
        "-Wno-unused-function".into(),
        "-Wno-implicit-fallthrough".into(),
        "-Wno-unused-but-set-variable".into(),
    ];

    println!("cargo:rerun-if-changed={}", src.display());
    let cc = env::var("CC").unwrap_or_else(|_| "gcc".into());
    let mut objs = Vec::with_capacity(objects.len());
    for c in objects {
        let cpath = src.join(c);
        let oname = format!("{}.o", c.trim_end_matches(".c"));
        let opath = out.join(&oname);
        println!("cargo:rerun-if-changed={}", cpath.display());
        let mut cmd = Command::new(&cc);
        cmd.arg("-c")
            .arg(&cpath)
            .arg("-o")
            .arg(&opath)
            .arg(format!("-I{}", src.display()))
            .args(&cflags);
        let status = cmd.status().unwrap_or_else(|e| {
            panic!("{cc} failed to launch for {c}: {e}");
        });
        if !status.success() {
            panic!("wasm3 compile failed: {c}");
        }
        objs.push(opath);
    }

    let lib = out.join("libm3.a");
    let ar = env::var("AR").unwrap_or_else(|_| "ar".into());
    let mut ar = Command::new(ar);
    ar.arg("rcs").arg(&lib).args(&objs);
    let status = ar.status().expect("ar launch");
    if !status.success() {
        panic!("wasm3 ar failed");
    }

    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=m3");
    println!("cargo:rustc-link-lib=m");
}
