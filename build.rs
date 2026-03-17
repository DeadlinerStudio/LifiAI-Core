fn main() {
    uniffi::generate_scaffolding("src/deadliner.udl").expect("FFI: UDL generation failed");
}
