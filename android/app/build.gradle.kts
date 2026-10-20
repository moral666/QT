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

// Relay de PRODUCAO, fixo na build release (o utilizador nao o ve nem o muda).
// Definir em android/gradle.properties (nao sao segredos: URL publico e chave PUBLICA):
//   qtRelayUrl=wss://3-250-10-20.sslip.io
//   qtRelayPin=<chave publica Noise do relay, 64 caracteres hex>
// ou nas variaveis de ambiente QT_RELAY_URL / QT_RELAY_PIN.
val relayUrl: String = (findProperty("qtRelayUrl") as String?) ?: System.getenv("QT_RELAY_URL") ?: ""
val relayPin: String = ((findProperty("qtRelayPin") as String?) ?: System.getenv("QT_RELAY_PIN") ?: "").lowercase()

// Notificacoes (FCM): valores do projeto Firebase, lidos de android/app/google-services.json
// (descarregado da consola do Firebase; NAO vai para o git). Sem o plugin google-services:
// a app inicia o Firebase a mao (Push.kt). Sem o ficheiro, compila sem notificacoes.
data class FcmConfig(val projectId: String, val appId: String, val apiKey: String, val senderId: String)
val fcm: FcmConfig? = run {
    val f = file("google-services.json")
    if (!f.exists()) return@run null
    @Suppress("UNCHECKED_CAST")
    val root = groovy.json.JsonSlurper().parse(f) as Map<String, Any?>
    val info = root["project_info"] as? Map<String, Any?> ?: return@run null
    val clients = root["client"] as? List<Map<String, Any?>> ?: return@run null
    val client = clients.firstOrNull {
        ((it["client_info"] as? Map<*, *>)?.get("android_client_info") as? Map<*, *>)?.get("package_name") == "sealer.app"
    } ?: return@run null
    val appId = (client["client_info"] as? Map<*, *>)?.get("mobilesdk_app_id") as? String ?: return@run null
    val apiKey = ((client["api_key"] as? List<*>)?.firstOrNull() as? Map<*, *>)?.get("current_key") as? String ?: return@run null
    FcmConfig(info["project_id"] as String, appId, apiKey, info["project_number"] as String)
}

android {
    namespace = "pt.qt.app"
    compileSdk = 36

    defaultConfig {
        // Identificador na Play Store (play.google.com/store/apps/details?id=sealer.app).
        // NUNCA mudar depois de publicar. O codigo Kotlin continua no pacote pt.qt.app (namespace).
        applicationId = "sealer.app"
        minSdk = 24            // igual ao `-P 24` do cargo-ndk
        // Android 16. Exigido pela Play Store para apps novas e atualizacoes.
        targetSdk = 36
        versionCode = 23
        versionName = "0.9.16"
        // So as 6 linguas da app (0.9.12): sem isto iam tambem as ~80 traducoes das bibliotecas
        // (AndroidX, Firebase, ZXing), que a app nunca mostra. A base (values/) e o portugues.
        @Suppress("DEPRECATION")
        resourceConfigurations += listOf("pt", "en", "fr", "es", "de", "it")
        // So as ABIs que o Rust compilou. Adiciona "x86_64" quando tiveres esse .so (emulador).
        ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a") }
    }

    buildFeatures { buildConfig = true }

    defaultConfig {
        buildConfigField("boolean", "FCM_ENABLED", (fcm != null).toString())
        buildConfigField("String", "FCM_PROJECT_ID", "\"${fcm?.projectId ?: ""}\"")
        buildConfigField("String", "FCM_APP_ID", "\"${fcm?.appId ?: ""}\"")
        buildConfigField("String", "FCM_API_KEY", "\"${fcm?.apiKey ?: ""}\"")
        buildConfigField("String", "FCM_SENDER_ID", "\"${fcm?.senderId ?: ""}\"")
    }

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
            // Debug: relay editavel na app. 10.0.2.2 = o PC visto de dentro do emulador.
            buildConfigField("boolean", "RELAY_EDITABLE", "true")
            buildConfigField("String", "RELAY_URL", "\"${relayUrl.ifBlank { "ws://10.0.2.2:9443" }}\"")
            buildConfigField("String", "RELAY_PIN", "\"$relayPin\"")
        }
        release {
            buildConfigField("boolean", "ALLOW_CLEARTEXT", "false")
            // Release: relay FIXO (sem ecra de configuracao); pinning obrigatorio.
            buildConfigField("boolean", "RELAY_EDITABLE", "false")
            buildConfigField("String", "RELAY_URL", "\"$relayUrl\"")
            buildConfigField("String", "RELAY_PIN", "\"$relayPin\"")
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
    // >= 5.17.0: libjnidispatch.so alinhado a paginas de 16 KB (JNA #1618/#1647).
    implementation("net.java.dev.jna:jna:5.17.0@aar")
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.8.1")

    implementation(platform("androidx.compose:compose-bom:2024.09.00"))
    implementation("androidx.compose.ui:ui")
    // libandroidx.graphics.path.so (dependencia do Compose): forca a versao mais
    // recente em vez da 1.0.1 de 2024 (alinhamento de 16 KB).
    implementation("androidx.graphics:graphics-path:1.1.0")
    implementation("androidx.compose.material3:material3")
    // Icones (seta, enviar, cadeado, +, partilhar). Versao vem do BOM do Compose.
    implementation("androidx.compose.material:material-icons-core")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.activity:activity-compose:1.9.2")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.6")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.6")
    // Bloqueio da app: impressao digital/rosto/PIN do telemovel (BiometricPrompt precisa de FragmentActivity).
    implementation("androidx.biometric:biometric:1.1.0")
    implementation("androidx.fragment:fragment-ktx:1.8.5")
    // Codigos QR: gerar (ZXing) e ler com a camara (ZXing Android Embedded). Tudo no
    // telemovel, sem servicos da Google nem rede.
    implementation("com.google.zxing:core:3.5.3")
    implementation("com.journeyapps:zxing-android-embedded:4.3.0")
    // Imagens e GIFs no chat: mostrar (Coil, com GIFs animados) e corrigir a rotacao
    // das fotos em Android 7-8 (ExifInterface). Sem cache em disco: ver SealerImages.
    implementation("io.coil-kt:coil-compose:2.7.0")
    implementation("io.coil-kt:coil-gif:2.7.0")
    implementation("androidx.exifinterface:exifinterface:1.3.7")
    // Notificacoes com a app fechada: so o Firebase Cloud Messaging (sem Analytics). Ver Push.kt.
    implementation(platform("com.google.firebase:firebase-bom:35.0.1"))
    implementation("com.google.firebase:firebase-messaging")

    // Testes unitarios (JVM, sem telemovel): logica pura em Kotlin. `gradle :app:testDebugUnitTest`
    testImplementation("junit:junit:4.13.2")
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
    if (wantsRelease && !relayUrl.startsWith("wss://")) {
        throw GradleException("Release sem relay: define qtRelayUrl=wss://... em android/gradle.properties (ou QT_RELAY_URL).")
    }
    if (wantsRelease && !Regex("^[0-9a-f]{64}$").matches(relayPin)) {
        throw GradleException("Release sem pinning: define qtRelayPin=<64 hex da chave publica Noise do relay> em android/gradle.properties (ou QT_RELAY_PIN).")
    }
    if (wantsRelease && android.signingConfigs.getByName("release").storeFile == null) {
        throw GradleException(
            "Release sem keystore: cria android/keystore.properties (ver android/README.md, " +
                "'Assinatura de release') ou define QT_KEYSTORE_FILE/QT_KEYSTORE_PASSWORD/QT_KEY_ALIAS/QT_KEY_PASSWORD."
        )
    }
}
