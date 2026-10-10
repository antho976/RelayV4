# Relay release R8 rules. Room, Hilt, WorkManager, Compose and kotlinx-serialization ship consumer
# rules for their generated code; these cover the seams they do not.

-keepattributes SourceFile,LineNumberTable
-renamesourcefileattribute SourceFile

# Enums read back from stored strings (route policies, outbox states).
-keepclassmembers enum * {
    <fields>;
    public static **[] values();
    public static ** valueOf(java.lang.String);
}

-keep class com.quietsoftware.relay.data.db.** { *; }
-keepclassmembers class com.quietsoftware.relay.** {
    *** Companion;
    kotlinx.serialization.KSerializer serializer(...);
}

-keep class * extends androidx.work.ListenableWorker { *; }
-keep @androidx.hilt.work.HiltWorker class * { <init>(...); }
-keep @dagger.assisted.AssistedFactory class * { *; }

-dontwarn kotlinx.coroutines.**
-dontwarn org.conscrypt.**
-dontwarn org.bouncycastle.**
-dontwarn org.openjsse.**
