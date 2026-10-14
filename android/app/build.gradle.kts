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
            // Esquemas de assinatura modernos. minSdk = 24, por isso o JAR signing (v1)
            // nao e necessario: v2 (APK Signature Scheme v2, Android 7+) assina o APK
            // inteiro incluindo os .so em lib/, e v3 (Android 9+) acrescenta suporte a
            // rotacao da chave de assinatura. v4 so serve para `adb install --incremental`.
            enableV1Signing = false
            enableV2Signing = true
            enableV3Signing = true
            enableV4Signing = false
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
    packaging {
        jniLibs {
            // .so guardados SEM compressao e alinhados a pagina dentro do APK (carregados
            // por mmap a partir do APK, sem extracao para disco). Evita o padrao "extrai
            // binario nativo e executa" que alguns motores heuristicos marcam.
            useLegacyPackaging = false
        }
        resources {
            // Metadados desnecessarios que so aumentam o APK.
            excludes += setOf("META-INF/*.kotlin_module", "META-INF/AL2.0", "META-INF/LGPL2.1", "DebugProbesKt.bin")
        }
    }
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

// Verificacao da assinatura do APK de release: `./gradlew :app:verifyReleaseSignature`
// Corre o apksigner das build-tools e falha se v2 OU v3 nao estiverem presentes.
tasks.register("verifyReleaseSignature") {
    group = "verification"
    description = "Verifica que o APK de release esta assinado com os esquemas v2 e v3."
    dependsOn("assembleRelease")
    val apkDir = layout.buildDirectory.dir("outputs/apk/release")
    val apksigner = android.sdkDirectory.resolve("build-tools/${android.buildToolsVersion}/apksigner")
    doLast {
        val apk = apkDir.get().asFile.listFiles()?.filter { it.name.endsWith(".apk") }?.singleOrNull()
            ?: throw GradleException("APK de release nao encontrado em ${apkDir.get().asFile}")
        val proc = ProcessBuilder(apksigner.path, "verify", "--verbose", "--print-certs", apk.path)
            .redirectErrorStream(true)
            .start()
        val text = proc.inputStream.bufferedReader().readText()
        val code = proc.waitFor()
        logger.lifecycle(text)
        if (code != 0) throw GradleException("apksigner verify falhou (codigo $code)")
        listOf("(APK Signature Scheme v2): true", "(APK Signature Scheme v3): true").forEach {
            if (!text.contains(it)) throw GradleException("Assinatura em falta: $it")
        }
    }
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
