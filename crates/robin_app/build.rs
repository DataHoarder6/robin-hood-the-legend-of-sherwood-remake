//! Build script: embed the Windows application manifest and icon resources
//! into the `robin` executable.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=windows/robin.rc");
    println!("cargo:rerun-if-changed=windows/robin.manifest");
    // A missing privilege/compatibility manifest must fail Windows builds.
    embed_resource::compile_for("windows/robin.rc", ["robin"], embed_resource::NONE)
        .manifest_required()
        .expect("cannot embed the Windows application manifest");
}
