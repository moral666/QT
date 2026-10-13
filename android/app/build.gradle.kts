import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

// Assinatura de RELEASE com keystore propria (NUNCA no repositorio). Le, por ordem:
// android/keystore.properties (storeFile, storePassword, keyAlias, keyPassword)
// ou as variaveis de ambiente QT_KEYSTORE_FILE / QT_KEYSTORE_PASSWORD / QT_KEY_ALIAS / QT_KEY_PASSWORD.
val keystoreProps = Properties().apply {
    val f = rootProject.file("keystore.properties")
    if (f.exists()) f.inputStream().use { load(it) }
}
fun signingProp(key: String, env: String): String? = keystoreProps.getProperty(key) ?: System.getenv(env)

android {
    namespace = "pt.qt.app"
    compileSdk = 34

    defaultConfig {
        applicationId = "pt.qt.app"
        minSdk = 24            // igual ao `-P 24` do cargo-ndk
        targetSdk = 34
        versionCode = 1
        versionName = "0.1-prototipo"
        // So as ABIs que o Rust compilou. Adiciona "x86_64" quando tiveres esse .so (emulador).
        ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a") }
    }

    buildFeatures { buildConfig = true }

    signingConfigs {
        create("release") {
            val path = signingProp("storeFile", "QT_KEYSTORE_FILE")
            if (path != null) {
                storeFile = rootProject.file(path)
                storePassword = signingProp("storePassword", "QT_KEYSTORE_PASSWORD")
                keyAlias = signingProp("keyAlias", "QT_KEY_ALIAS")
                keyPassword = signingProp("keyPassword", "QT_KEY_PASSWORD")
            }
        }
    }

    buildTypes {
        debug {
            // So a build debug pode falar ws:// (ver src/debug/res/xml/network_security_config.xml).
            buildConfigField("boolean", "ALLOW_CLEARTEXT", "true")
        }
        release {
            buildConfigField("boolean", "ALLOW_CLEARTEXT", "false")
            isDebuggable = false
            signingConfig = signingConfigs.getByName("release")
            isMinifyEnabled = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    buildFeatures { compose = true }
    packaging { jniLibs { useLegacyPackaging = false } }
}

dependencies {
    // UniFFI (Kotlin) carrega libqt_ffi.so via JNA - exige o artefacto AAR
    implementation("net.java.dev.jna:jna:5.14.0@aar")
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.8.1")

    implementation(platform("androidx.compose:compose-bom:2024.09.00"))
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.activity:activity-compose:1.9.2")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.6")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.6")
}

// Falha cedo (e claramente) se se tentar gerar um release sem keystore propria,
// em vez de produzir um APK assinado com a chave de debug ou nao assinado.
gradle.taskGraph.whenReady {
    val wantsRelease = allTasks.any {
        it.project == project && it.name.matches(Regex("(assemble|bundle|package)Release.*"))
    }
    if (wantsRelease && android.signingConfigs.getByName("release").storeFile == null) {
        throw GradleException(
            "Release sem keystore: cria android/keystore.properties (ver android/README.md, " +
                "'Assinatura de release') ou define QT_KEYSTORE_FILE/QT_KEYSTORE_PASSWORD/QT_KEY_ALIAS/QT_KEY_PASSWORD."
        )
    }
}
