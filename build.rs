fn main() {
    println!("cargo:rerun-if-changed=linker/x86_64.ld");
    println!("cargo:rerun-if-changed=linker/aarch64.ld");
}
