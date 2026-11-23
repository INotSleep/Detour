plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "dev.detour.android"
    compileSdk = 37

    defaultConfig {
        applicationId = "dev.detour.android"
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = "0.6.0"
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    buildFeatures { compose = true }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    packaging { jniLibs { useLegacyPackaging = false } }
}

kotlin { jvmToolchain(21) }

dependencies {
    val compose = platform("androidx.compose:compose-bom:2026.09.00")
    implementation(compose)
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("androidx.activity:activity-compose:1.12.4")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.10.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.10.0")
    implementation("androidx.core:core-ktx:1.17.0")
    implementation("androidx.biometric:biometric:1.1.0")
    // BiometricPrompt needs a FragmentActivity.
    implementation("androidx.fragment:fragment-ktx:1.8.9")
    // Drive sign-in pages in a Custom Tab.
    implementation("androidx.browser:browser:1.9.0")
    // The other device's QR, scanned by Google's scanner (no camera permission).
    implementation("com.google.android.gms:play-services-code-scanner:16.1.0")
    // The QR this device shows for another one to join sync.
    implementation("com.google.zxing:core:3.5.3")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    implementation(project(":terminal"))
    // The core's Kotlin side (UniFFI) reaches it through JNA.
    implementation("net.java.dev.jna:jna:5.18.1@aar")
}

// The app's text in every language: the iOS app's Localizable.strings, shared.
abstract class CopyL10n : DefaultTask() {
    @get:InputDirectory abstract val source: DirectoryProperty
    @get:OutputDirectory abstract val output: DirectoryProperty

    @TaskAction fun copy() {
        val out = output.get().asFile
        out.deleteRecursively()
        source.get().asFile.listFiles { f -> f.name.endsWith(".lproj") }?.forEach { dir ->
            val strings = File(dir, "Localizable.strings")
            if (strings.exists()) strings.copyTo(File(out, "${dir.name}/Localizable.strings"), overwrite = true)
        }
    }
}

val copyL10n = tasks.register<CopyL10n>("copyL10n") {
    source.set(layout.projectDirectory.dir("../../ios/Detour/Resources"))
}

androidComponents {
    onVariants { variant -> variant.sources.assets?.addGeneratedSourceDirectory(copyL10n, CopyL10n::output) }
}
