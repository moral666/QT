# UniFFI + JNA usam reflexao/JNI: nao ofuscar.
-keep class com.sun.jna.** { *; }
-keep class uniffi.qt_ffi.** { *; }
-dontwarn java.awt.**
