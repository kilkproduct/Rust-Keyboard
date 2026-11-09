# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.
#
# For more details, see
#   http://developer.android.com/guide/developing/tools/proguard.html

# If your project uses WebView with JS, uncomment the following
# and specify the fully qualified class name to the JavaScript interface
# class:
#-keepclassmembers class fqcn.of.javascript.interface.for.webview {
#   public *;
#}

# Uncomment this to preserve the line number information for
# debugging stack traces.
-keepattributes SourceFile,LineNumberTable

# If you keep the line number information, uncomment this to
# hide the original source file name.
#-renamesourcefileattribute SourceFile

-keepclasseswithmembernames,includedescriptorclasses class * {
    native <methods>;
}

-keepclasseswithmembers class * {
    native <methods>;
}
# Keep classes and methods that have the @UsedForTesting annotation
-keep @org.futo.inputmethod.annotations.UsedForTesting class *
-keepclassmembers class * {
    @org.futo.inputmethod.annotations.UsedForTesting *;
}

# Keep classes and methods that have the @ExternallyReferenced annotation
-keep @org.futo.inputmethod.annotations.ExternallyReferenced class *
-keepclassmembers class * {
    @org.futo.inputmethod.annotations.ExternallyReferenced *;
}

# Keep native methods
-keepclassmembers class * {
    native <methods>;
}

# Keep classes that are used as a parameter type of methods that are also marked as keep
# to preserve changing those methods' signature.
-keep class org.futo.inputmethod.latin.AssetFileAddress
-keep class org.futo.inputmethod.latin.Dictionary
-keep class org.futo.inputmethod.latin.NgramContext
-keep class org.futo.inputmethod.latin.makedict.ProbabilityInfo
-keep class org.futo.inputmethod.keyboard.KeyboardLayout { *; }

-optimizations !code/allocation/variable

# JSR 305 annotations are for embedding nullability information.
-dontwarn javax.annotation.**

# Keep the four classes the native library registers methods on. They declare
# their methods `native`, which the rules above already keep, but the names are
# also looked up by the engine itself (`JNI_OnLoad` calls FindClass with the
# fully qualified name), so they are pinned here instead of relying on the
# side effect of a keep rule.
-keep class org.futo.inputmethod.latin.BinaryDictionary { *; }
-keep class org.futo.inputmethod.keyboard.ProximityInfo { *; }
-keep class org.futo.inputmethod.latin.DicTraverseSession { *; }
-keep class org.futo.inputmethod.latin.utils.BinaryDictionaryUtils { *; }

# Repackaging every class into the default package and letting R8 widen access
# was tried for size and reverted: it is the one optimisation in this file that
# changes the shape of the application for code the shrinker cannot see (name
# based lookups, serialised class names, the manifest). The megabytes it saved
# came from assets; the ~100 KB it saves here are not worth a crash on startup.

# Strip logging calls from the release build.
-assumenosideeffects class android.util.Log {
    public static *** d(...);
    public static *** v(...);
    public static *** i(...);
    public static *** w(...);
    public static *** e(...);
}
