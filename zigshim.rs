use std::path::Path;
use std::process::exit;

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let to = dst.join(e.file_name());
        let ft = e.file_type().unwrap();
        if ft.is_dir() {
            copy_dir(&e.path(), &to);
        } else if ft.is_file() {
            let _ = std::fs::copy(e.path(), &to);
        }
    }
}

fn is_abs(p: &str) -> bool {
    let b = p.as_bytes();
    (b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic()) || p.starts_with("\\\\")
}

fn main() {
    unsafe {
        std::env::set_var("ZIG_GLOBAL_CACHE_DIR", "D:\\temp\\zig-global");
        std::env::set_var(
            "ZIG_LOCAL_CACHE_DIR",
            format!("D:\\temp\\zig-local\\{}", std::process::id()),
        );
    }
    let exe = std::env::current_exe().unwrap();
    let stem = exe.file_stem().unwrap().to_string_lossy().to_lowercase();
    let sub = if stem.contains("cxx") {
        "c++"
    } else if stem.contains("ranlib") {
        "ranlib"
    } else if stem.contains("ar") {
        "ar"
    } else {
        "cc"
    };
    let args: Vec<String> = std::env::args()
        .skip(1)
        .flat_map(|a| {
            if let Some(p) = a.strip_prefix('@') {
                if let Ok(text) = std::fs::read_to_string(p) {
                    return text
                        .split_whitespace()
                        .map(|w| w.trim_matches('"').to_string())
                        .collect();
                }
            }
            vec![a]
        })
        .collect();
    if matches!(sub, "ar" | "ranlib") {
        let st = std::process::Command::new("C:\\Users\\xarle\\zig\\zig.exe")
            .arg(sub)
            .args(&args)
            .status()
            .unwrap();
        exit(st.code().unwrap_or(1));
    }
    let cwd = std::env::current_dir().unwrap();
    let cwd_str = cwd.to_string_lossy().replace('/', "\\");
    let has_rel_input = args.iter().any(|a| {
        let t = a.as_str();
        (t.ends_with(".c") || t.ends_with(".cc") || t.ends_with(".cpp") || t.ends_with(".S"))
            && !is_abs(t)
    });
    let mirror: String = if has_rel_input
        && (cwd_str.starts_with("C:\\") || cwd_str.starts_with("c:\\"))
    {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in cwd_str.bytes() {
            h = (h ^ b as u64).wrapping_mul(1099511628211);
        }
        let m = format!("D:\\temp\\zigwrap\\{:016x}", h);
        if !Path::new(&m).exists() {
            let tmp = format!("{}.tmp{}", m, std::process::id());
            copy_dir(&cwd, Path::new(&tmp));
            if !Path::new(&m).exists() {
                let _ = std::fs::rename(&tmp, &m);
            } else {
                let _ = std::fs::remove_dir_all(&tmp);
            }
        }
        m
    } else {
        cwd_str
    };
    let map = |v: &str| -> String {
        if is_abs(v) {
            v.to_string()
        } else {
            format!("{}\\{}", mirror, v)
        }
    };
    const SELF_CONTAINED: &str = "D:\\rustup\\toolchains\\stable-x86_64-pc-windows-gnu\\lib\\rustlib\\x86_64-pc-windows-gnu\\lib\\self-contained";
    let is_link = !args.iter().any(|a| a == "-c");
    let mut out: Vec<String> = vec![
        sub.into(),
        "-target".into(),
        "x86_64-windows-gnu".into(),
        "-fno-sanitize=all".into(),
    ];
    let mut it = args.iter().peekable();
    while let Some(a) = it.next() {
        let t = a.as_str();
        if t == "-?" || t.starts_with("--target=") {
            continue;
        }
        if t == "-target" {
            it.next();
            continue;
        }
        if t == "-c" || t == "-isystem" || t == "-include" || t == "-I" {
            out.push(a.clone());
            if let Some(v) = it.next() {
                out.push(map(v));
            }
            continue;
        }
        if t.starts_with("-I") && t.len() > 2 {
            out.push(format!("-I{}", map(&t[2..])));
            continue;
        }
        if is_link {
            if let Some(d) = t.strip_prefix("-Wl,") {
                if d.ends_with(".def") {
                    continue;
                }
            }
            if t.ends_with(".def") {
                continue;
            }
            if let Some(name) = t.strip_prefix("-l:") {
                let f = format!("{}\\{}", SELF_CONTAINED, name);
                if Path::new(&f).exists() {
                    out.push(f);
                }
                continue;
            }
            if t.ends_with(".def") && is_abs(t) {
                continue;
            }
            if let Some(name) = t.strip_prefix("-l") {
                let f = format!("{}\\lib{}.a", SELF_CONTAINED, name);
                if Path::new(&f).exists() {
                    out.push(f);
                    continue;
                }
                let w = format!("D:\\temp\\winapi-shim\\lib{}.dll.a", name);
                if Path::new(&w).exists() {
                    out.push(w);
                    continue;
                }
                let u = format!("D:\\temp\\ubsan-stub\\lib{}.a", name);
                if Path::new(&u).exists() {
                    out.push(u);
                    continue;
                }
            }
        }
        out.push(a.clone());
    }
    let st = std::process::Command::new("C:\\Users\\xarle\\zig\\zig.exe")
        .args(&out)
        .current_dir(&mirror)
        .status()
        .unwrap();
    exit(st.code().unwrap_or(1));
}
