fn main() {
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("vendored protoc");
    unsafe {
        std::env::set_var("PROTOC", &protoc);
    }
    tonic_build::compile_protos("proto/slow.proto").expect("compile slow.proto");
}
