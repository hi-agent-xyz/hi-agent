// The Android client is a standalone Gradle build, not a module of anything
// else in this repository. It does not link the Rust core, `hi-app`, or
// `hi-wire` — it speaks the documented HTTP client API and nothing more, the
// same independence the iOS target has.
pluginManagement {
    repositories {
        google {
            content {
                includeGroupByRegex("com\\.android.*")
                includeGroupByRegex("com\\.google.*")
                includeGroupByRegex("androidx.*")
            }
        }
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "HiAgentAndroid"
include(":app")
