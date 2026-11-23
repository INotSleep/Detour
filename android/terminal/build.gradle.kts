// Termux's terminal emulator and view (Apache License 2.0, from Jack Palevich's Terminal
// Emulator for Android), v0.118.1, with the session fed by Detour's SSH core instead of a local
// process: see TerminalSession.java and NOTICE.
plugins {
    id("com.android.library")
}

android {
    namespace = "com.termux.view"
    compileSdk = 37
    defaultConfig { minSdk = 26 }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    implementation("androidx.annotation:annotation:1.9.1")
}
