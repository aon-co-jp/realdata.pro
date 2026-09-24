fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo が設定する");
    let out = std::env::var("OUT_DIR").expect("cargo が設定する");
    opencuda_shader_build::compile_one(
        format!("{dir}/shaders/sgemm_naive.comp"),
        format!("{out}/sgemm_naive.spv"),
    );
}
