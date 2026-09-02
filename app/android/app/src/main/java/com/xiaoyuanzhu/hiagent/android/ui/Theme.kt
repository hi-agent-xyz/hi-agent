package com.xiaoyuanzhu.hiagent.android.ui

import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Hub
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/**
 * One place for the app's visual language, so a screen never invents its own
 * spacing scale, corner radius, or status colour.
 *
 * The face a core renders is a web page we do not control. Everything the native
 * shell draws around it is deliberately quiet: matte surfaces, one accent, no
 * gloss — the chrome should recede until the moment it is needed.
 *
 * Deliberately **not** Material You dynamic colour. The mark is a fixed pair of
 * brand colours, and a shell tinted from the person's wallpaper would put a
 * different accent beside it on every handset.
 */
object Theme {
    /** The brand ink, lifted in dark so it still reads as interactive. */
    val inkLight = Color(0xFF0E6E86)
    val inkDark = Color(0xFF5FC8E0)

    val cardRadius: Dp = 20.dp
    val tileRadius: Dp = 16.dp
    val gutter: Dp = 24.dp

    val ink: Color
        @Composable get() = if (isSystemInDarkTheme()) inkDark else inkLight

    /** Barely-there card edge; lifted in dark where a shadow cannot separate. */
    val hairline: Color
        @Composable get() = if (isSystemInDarkTheme()) {
            Color.White.copy(alpha = 0.10f)
        } else {
            Color.Black.copy(alpha = 0.06f)
        }

    /** A very low-energy wash behind native screens. A surface, not a decoration. */
    val canvas: Brush
        @Composable get() = Brush.verticalGradient(
            if (isSystemInDarkTheme()) {
                listOf(Color(0xFF10141A), Color(0xFF10141A))
            } else {
                listOf(Color(0xFFFFFFFF), Color(0xFFEFF2F6))
            },
        )
}

@Composable
fun HiAgentTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val colors = if (dark) {
        darkColorScheme(
            primary = Theme.inkDark,
            onPrimary = Color(0xFF00202B),
            background = Color(0xFF10141A),
            surface = Color(0xFF171C24),
        )
    } else {
        lightColorScheme(
            primary = Theme.inkLight,
            onPrimary = Color.White,
            background = Color(0xFFF7F8FA),
            surface = Color.White,
        )
    }
    MaterialTheme(colorScheme = colors, content = content)
}

/** Full-bleed app canvas. Applied once per native screen. */
@Composable
fun Modifier.hiCanvas(): Modifier = this
    .fillMaxSize()
    .background(Theme.canvas)

/**
 * The status dot next to a core's name. It breathes only while the core is
 * answering; a still dot means the app is not claiming anything it hasn't just
 * checked.
 */
@Composable
fun StatusDot(
    state: com.xiaoyuanzhu.hiagent.android.core.HealthState,
    diameter: Dp = 9.dp,
    modifier: Modifier = Modifier,
) {
    val tint = state.tint()
    Box(
        modifier = modifier.size(diameter * 2.1f),
        contentAlignment = Alignment.Center,
    ) {
        if (state.isLive) {
            val transition = rememberInfiniteTransition(label = "breath")
            val scale by transition.animateFloat(
                initialValue = 0.55f,
                targetValue = 1f,
                animationSpec = infiniteRepeatable(
                    animation = tween(2200),
                    repeatMode = RepeatMode.Restart,
                ),
                label = "scale",
            )
            val alpha by transition.animateFloat(
                initialValue = 1f,
                targetValue = 0f,
                animationSpec = infiniteRepeatable(
                    animation = tween(2200),
                    repeatMode = RepeatMode.Restart,
                ),
                label = "alpha",
            )
            Box(
                Modifier
                    .size(diameter * 2.1f * scale)
                    .clip(CircleShape)
                    .background(tint.copy(alpha = 0.28f * alpha)),
            )
        }
        Box(Modifier.size(diameter).clip(CircleShape).background(tint))
    }
}

/**
 * The dot colour. Only "here" earns green; everything else stays grey or amber,
 * so a roster of healthy cores is calm rather than a traffic light.
 */
@Composable
fun com.xiaoyuanzhu.hiagent.android.core.HealthState.tint(): Color = when (this) {
    com.xiaoyuanzhu.hiagent.android.core.HealthState.HERE -> Color(0xFF34C759)
    com.xiaoyuanzhu.hiagent.android.core.HealthState.ASLEEP -> Color(0xFF5E5CE6)
    com.xiaoyuanzhu.hiagent.android.core.HealthState.UNREACHABLE -> Color(0xFFFF9500)
    else -> MaterialTheme.colorScheme.onSurfaceVariant
}

/**
 * The product mark. The sealed logo is a raster asset used as the launcher icon;
 * in-app this is its stand-in at sizes where the tile would be the wrong shape —
 * the same three-node glyph the iOS shell draws, not a second brand mark.
 */
@Composable
fun CoreMark(size: Dp = 88.dp, modifier: Modifier = Modifier) {
    Box(
        modifier = modifier
            .size(size)
            .clip(RoundedCornerShape(size * 0.28f))
            .background(Theme.ink.copy(alpha = 0.10f)),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            imageVector = Icons.Rounded.Hub,
            contentDescription = null,
            tint = Theme.ink,
            modifier = Modifier.size(size * 0.46f),
        )
    }
}

/**
 * A matte card, used for grouped content instead of a list style that drags its
 * own background in and fights the canvas.
 */
@Composable
fun Card(
    modifier: Modifier = Modifier,
    padding: Dp = 18.dp,
    content: @Composable ColumnScope.() -> Unit,
) {
    Surface(
        modifier = modifier
            .clip(RoundedCornerShape(Theme.cardRadius))
            .border(1.dp, Theme.hairline, RoundedCornerShape(Theme.cardRadius)),
        color = MaterialTheme.colorScheme.surface,
        shape = RoundedCornerShape(Theme.cardRadius),
    ) {
        androidx.compose.foundation.layout.Column(Modifier.padding(padding), content = content)
    }
}
