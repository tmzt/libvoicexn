fn main() {
    // Writes target/aarch64-linux-android/<profile>/gradle/libvoicexn/, the
    // whole Gradle project. The app moves to examples/voicexn (plan
    // 20261022_05, step 1), which scripts/android.sh packages.
    let mut project = libmorph_build::gradle_project("dev.tmzt.voicexn")
        .label("voicexn")
        .lib_name("libvoicexn");
    // libmorph asks for it after init once the manifest declares it.
    if std::env::var_os("CARGO_FEATURE_STT_MOSHI_XN").is_some() {
        project = project.permission("android.permission.RECORD_AUDIO");
    }
    project
        .generate()
        .expect("libmorph-build: generating the Gradle project");
}
