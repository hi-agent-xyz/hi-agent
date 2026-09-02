package com.xiaoyuanzhu.hiagent.android

import android.app.Application

/**
 * Nothing to set up at process start. The class exists so the manifest names a
 * stable application entry the app can grow into — a WebView data-directory
 * suffix, a crash reporter, a strict-mode policy in debug — without a manifest
 * change becoming part of that work.
 */
class HiAgentApplication : Application()
