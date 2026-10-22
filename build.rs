fn main() {
    // Writes target/aarch64-linux-android/<profile>/gradle/voicexn/, the
    // whole Gradle project that scripts/android.sh packages.
    libmorph_build::gradle_project("dev.tmzt.voicexn")
        .label("voicexn")
        .lib_name("voicexn")
        .generate()
        .expect("libmorph-build: generating the Gradle project");
}
