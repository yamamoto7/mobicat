plugins {
    id("com.android.application")
}

android {
    namespace = "com.example.shelf"
    compileSdk = 36

    defaultConfig {
        // -PappId=… builds look-alike apps so the app picker has neighbors.
        applicationId = providers.gradleProperty("appId").getOrElse("com.example.shelf")
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = "1.0"
    }
}
