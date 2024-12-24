use cc;

// Compile the C TUN wrappers into a library that will be linked to the Rust
// code.
fn main() {
    println!("cargo::rerun-if-changed=src/tun.c");
    cc::Build::new()
        .file("src/tun.c")
        .compile("tun");
}
