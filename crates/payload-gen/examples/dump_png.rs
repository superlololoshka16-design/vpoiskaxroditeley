fn main() {
    let png = payload_gen::png_bytes_pixels(1, 5, &[]);
    eprintln!("len={}", png.len());
    eprintln!("hex={}", png.iter().map(|b| format!("{:02x}", b)).collect::<String>());
}
