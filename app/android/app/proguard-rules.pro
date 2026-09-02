# The JS bridge is called by name from injected JavaScript, so R8 cannot see the
# call site and would otherwise rename or strip the method.
-keepclassmembers class com.xiaoyuanzhu.hiagent.android.web.SessionBridge {
    @android.webkit.JavascriptInterface <methods>;
}

# OkHttp ships its own consumer rules; these silence the optional-dependency
# warnings its platform detection triggers under full-mode R8.
-dontwarn okhttp3.internal.platform.**
-dontwarn org.conscrypt.**
-dontwarn org.bouncycastle.**
-dontwarn org.openjsse.**
