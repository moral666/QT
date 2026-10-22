# =============================================================================
# QT - regras R8/ProGuard (build release, isMinifyEnabled = true)
#
# Ponte nativa: Kotlin (bindings UniFFI gerados) -> JNA -> libjnidispatch.so
#               -> libqt_ffi.so (Rust: X3DH/PQXDH, Double Ratchet, SQLCipher)
#
# O JNA resolve classes, campos e metodos por REFLEXAO e pelo NOME: se o R8
# renomear ou remover algo destas classes, a app compila mas rebenta em
# runtime (UnsatisfiedLinkError / NoSuchFieldError / "Structure.getFieldOrder").
# =============================================================================

# ---------- Atributos necessarios a reflexao do JNA/UniFFI ----------
-keepattributes *Annotation*, Signature, InnerClasses, EnclosingMethod, Exceptions, RuntimeVisibleAnnotations

# ---------- JNA (net.java.dev.jna:jna:...@aar, inclui libjnidispatch.so) ----------
# Classes internas do JNA: carregadas pelo libjnidispatch.so via JNI pelo nome.
-keep class com.sun.jna.** { *; }
-keepclassmembers class com.sun.jna.** { *; }

# Qualquer interface de biblioteca nativa (as funcoes sao ligadas pelo NOME do metodo).
-keep interface * extends com.sun.jna.Library { *; }
-keep class * implements com.sun.jna.Library { *; }

# Structures: o JNA le os campos por reflexao, na ordem de @FieldOrder.
-keep class * extends com.sun.jna.Structure { *; }
-keepclassmembers class * extends com.sun.jna.Structure { <fields>; <init>(...); }

# Callbacks Rust -> Kotlin (callback interfaces / foreign traits do UniFFI).
-keep interface * extends com.sun.jna.Callback { *; }
-keep class * implements com.sun.jna.Callback { *; }

# PointerType / IntegerType / NativeMapped usados pelos bindings.
-keep class * extends com.sun.jna.PointerType { *; }
-keep class * extends com.sun.jna.IntegerType { *; }
-keep class * implements com.sun.jna.NativeMapped { *; }

# Qualquer metodo `native` (JNI direto) mantem o nome.
-keepclasseswithmembernames,includedescriptorclasses class * {
    native <methods>;
}

# O JNA tem caminhos de codigo para desktop (AWT) que nao existem em Android.
-dontwarn java.awt.**

# ---------- Bindings UniFFI gerados (ffi/generate_bindings.sh -> uniffi/qt_ffi/qt_ffi.kt) ----------
# Inclui RustBuffer, ForeignBytes, UniffiRustCallStatus, a interface UniffiLib
# (nomes dos simbolos exportados por libqt_ffi.so), checksums e conversores.
-keep class uniffi.** { *; }
-keepclassmembers class uniffi.** { *; }
-keepnames class uniffi.**

# ---------- SQLCipher ----------
# Na arquitetura atual o SQLCipher NAO existe do lado Java/Kotlin: e compilado
# DENTRO de libqt_ffi.so (rusqlite, feature `bundled-sqlcipher-vendored-openssl`)
# e acedido so pelo Rust. Nao ha classes para proteger e nao e preciso
# adicionar a dependencia Java do SQLCipher.
#
# Se um dia passares a usar o SQLCipher do lado Android (net.zetetic:sqlcipher-android),
# descomenta:
# -keep class net.zetetic.database.** { *; }
# -keep class net.zetetic.database.sqlcipher.** { *; }
# -keepclassmembers class net.zetetic.database.** { native <methods>; }
# -dontwarn net.zetetic.database.**

# ---------- Ofuscacao (opcional, recomendado para este projeto) ----------
# O codigo-fonte e publico (AGPL), por isso renomear classes nao protege nada;
# alem disso, codigo fortemente ofuscado + binarios nativos e um dos padroes
# que motores antivirus heuristicos associam a malware. Manter o shrinking
# (remove codigo morto) e desligar so a renomeacao torna o APK mais "legivel"
# para esses motores e os stack traces mais uteis. Descomenta para ativar:
# -dontobfuscate

# ---------- Registos (Log) na release ----------
# A release nao escreve registos de rotina (Log.v/d/i): o R8 apaga as chamadas.
# Ficam so os avisos (Log.w/e), sem dados das mensagens nem chaves, porque sao
# a unica forma de ver problemas de ligacao (`adb logcat -s SealerRelay`), ja
# que a app nao mostra avisos de ligacao. A build debug mantem tudo.
-assumenosideeffects class android.util.Log {
    public static int v(...);
    public static int d(...);
    public static int i(...);
}
