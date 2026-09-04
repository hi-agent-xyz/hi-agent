package com.xiaoyuanzhu.hiagent.android

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.viewModels
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Surface
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import com.xiaoyuanzhu.hiagent.android.ui.HiAgentTheme
import com.xiaoyuanzhu.hiagent.android.ui.TvContentScreen

/**
 * The television's entry point.
 *
 * Two things the handset activity does are deliberately absent. There is no
 * `enableEdgeToEdge()`, because the inset it exists to expose is a display
 * cutout and a television has none — what a TV has instead is overscan, which
 * the system cannot measure and therefore never reports, so it is handled as a
 * fixed margin in `Tv.overscan` and in the face's `tv` shape rather than read
 * from the window. And there is no `hiagent://pair` handling, because nothing on
 * a television can deliver that link: pairing here is typed.
 *
 * The screen is also never held awake. A face left up in an empty room is a
 * still image on a panel that may be OLED, and the system's own screensaver is
 * the right answer to that. Waking the room's television to say something is a
 * different capability that does not exist yet, on any client.
 */
class MainActivity : ComponentActivity() {
    private val model: AppModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        setContent {
            HiAgentTheme {
                Surface(
                    modifier = Modifier.fillMaxSize(),
                    color = Color.Transparent,
                ) {
                    TvContentScreen(model)
                }
            }
        }
    }
}
