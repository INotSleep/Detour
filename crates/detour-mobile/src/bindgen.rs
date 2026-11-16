//! `cargo run -p detour-mobile --features bindgen --bin uniffi-bindgen -- generate ...`:
//! the Swift side of this crate.
fn main() {
    uniffi::uniffi_bindgen_main()
}
