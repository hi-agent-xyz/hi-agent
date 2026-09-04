// AGP 9 has built-in Kotlin support, so `org.jetbrains.kotlin.android` is not
// applied here — applying it is an error, not a redundancy.
plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

// One version for the whole product. `../../../VERSION` is the source of truth
// the same way it is for Cargo.toml and the iOS project file; `make bump-version`
// keeps this line in step and `scripts/check-version.sh` fails the build if it
// drifts.
val hiAgentVersion = "0.1.0"

// Derived, never stamped: Play and every installer want a monotonic integer, and
// deriving it from the semver means one file to bump instead of two that can
// disagree. Room for 100 patches and 100 minors per major.
val hiAgentVersionCode = hiAgentVersion.split(".", "-")
    .take(3)
    .map { it.toIntOrNull() ?: 0 }
    .let { (major, minor, patch) -> major * 10000 + minor * 100 + patch }

android {
    namespace = "com.xiaoyuanzhu.hiagent.android"
    compileSdk = 37

    defaultConfig {
        applicationId = "com.xiaoyuanzhu.hiagent.android"
        // Android 9. The floor is set by two things the shell depends on rather
        // than by taste: Keystore's `setUnlockedDeviceRequired`, which is what
        // makes the credential's at-rest guarantee comparable to the iOS
        // Keychain's `AfterFirstUnlockThisDeviceOnly`, and the network security
        // config being the settled way to say what cleartext is allowed.
        minSdk = 28
        targetSdk = 37
        versionCode = hiAgentVersionCode
        versionName = hiAgentVersion
    }

    // One app, one `applicationId`, two device shapes. A flavor rather than a
    // second Gradle build because the split is only the shell: the client layer
    // under `core/` — the roster, the Keystore credential, session exchange, and
    // above all `normalizeBaseUrl`'s cleartext policy and its eight tests — is the
    // same code on a handset and on a television, and a second copy of a security
    // policy is a copy that drifts.
    //
    // The `applicationId` is deliberately *not* suffixed per flavor: no device is
    // both a phone and a television, so the two installs never meet. (Publishing
    // both to Play as multiple APKs under one listing would need distinct
    // version codes; that is a distribution question and Play is not the intended
    // channel — see `docs/platforms/android.md`.)
    flavorDimensions += "device"

    productFlavors {
        create("mobile") {
            dimension = "device"
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
        debug {
            applicationIdSuffix = ".debug"
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
    }

    packaging {
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.lifecycle.runtime.ktx)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.activity.compose)

    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.ui.graphics)
    implementation(libs.androidx.compose.ui.tooling.preview)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.material.icons.extended)
    debugImplementation(libs.androidx.compose.ui.tooling)

    implementation(libs.androidx.webkit)
    implementation(libs.androidx.datastore.preferences)
    implementation(libs.okhttp)

    // Scanning is CameraX frames decoded by ZXing, deliberately not ML Kit:
    // ML Kit's barcode scanner pulls in Google Play services, which is exactly
    // the dependency that does not exist on the handsets this app is for.
    //
    // Scoped to the handset flavor, because a QR is read with a camera and a
    // television has none. This is the flavor split doing its job: the code that
    // cannot work on a device shape is not merely unreachable there, it is not
    // in the APK.
    "mobileImplementation"(libs.androidx.camera.core)
    "mobileImplementation"(libs.androidx.camera.camera2)
    "mobileImplementation"(libs.androidx.camera.lifecycle)
    "mobileImplementation"(libs.androidx.camera.view)
    "mobileImplementation"(libs.zxing.core)

    testImplementation(libs.junit)
    testImplementation(libs.kotlin.test)
}
