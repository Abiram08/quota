//! Compiles the Slint UI (`ui/app.slint`) into Rust at build time.
fn main() {
    slint_build::compile("ui/app.slint").expect("Slint UI failed to compile");
}
