use std::env;
use std::path::PathBuf;
use tonic_prost_build::configure;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set by cargo"));
    let descriptor_path = out_dir.join("stats_descriptor.bin");

    configure()
        .file_descriptor_set_path(&descriptor_path)
        .compile_protos(&["proto/stats.proto"], &["proto/stats"])
        .expect("Could not compile protos");
}
