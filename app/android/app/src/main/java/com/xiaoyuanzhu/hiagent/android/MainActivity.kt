package com.xiaoyuanzhu.hiagent.android

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Surface
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import com.xiaoyuanzhu.hiagent.android.ui.ContentScreen
import com.xiaoyuanzhu.hiagent.android.ui.HiAgentTheme

class MainActivity : ComponentActivity() {
    private val model: AppModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        // Edge to edge, and no automatic inset fitting. The face reads
        // `env(safe-area-inset-*)` itself and holds its own content clear; if
        // the window also inset the WebView, the notch would be subtracted
        // twice. WebView only fills those CSS variables when the window is
        // laid out under the cutout, which is what `shortEdges` in the theme
        // and this call together arrange.
        enableEdgeToEdge()

        setContent {
            HiAgentTheme {
                Surface(
                    modifier = Modifier.fillMaxSize(),
                    color = Color.Transparent,
                ) {
                    ContentScreen(model)
                }
            }
        }

        handlePairingIntent(intent)
    }

    /**
     * `singleTask` in the manifest means a `hiagent://pair` link arriving while
     * the app is already running lands here rather than starting a second copy —
     * which is what makes scanning from the Camera app work when Hi Agent is
     * already open behind it.
     */
    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handlePairingIntent(intent)
    }

    private fun handlePairingIntent(intent: Intent?) {
        val uri = intent?.data ?: return
        if (intent.action != Intent.ACTION_VIEW) return
        model.handleIncomingUri(uri)
    }
}
