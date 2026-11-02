fn main() {
    // Add Swift runtime library path so the dynamic linker can find
    // libswift_Concurrency.dylib and other Swift libraries at runtime.
    // Required for the foundation-models crate which links against Swift.
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}
