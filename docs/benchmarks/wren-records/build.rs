use std::{env, fs, path::PathBuf};
fn main() {
    let root = PathBuf::from(
        env::var_os("WREN_SOURCE_DIR").expect("Set WREN_SOURCE_DIR to Wren 0.4.0 sources"),
    );
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let source = fs::read_to_string("records.wren").unwrap();
    let escaped = source
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    fs::write(
        output.join("records_script.h"),
        format!("static const char* script = \"{escaped}\";\n"),
    )
    .unwrap();
    let mut files: Vec<_> = fs::read_dir(root.join("src/vm"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "c"))
        .collect();
    files.sort();
    cc::Build::new()
        .files(files)
        .file("bridge.c")
        .include(root.join("src/include"))
        .include(root.join("src/vm"))
        .include(output)
        .define("WREN_OPT_META", "0")
        .define("WREN_OPT_RANDOM", "0")
        .warnings(false)
        .compile("wren_records");
    println!("cargo:rerun-if-env-changed=WREN_SOURCE_DIR");
    println!("cargo:rerun-if-changed={}", root.join("src").display());
    for file in ["bridge.c", "records.wren"] {
        println!("cargo:rerun-if-changed={file}");
    }
}
