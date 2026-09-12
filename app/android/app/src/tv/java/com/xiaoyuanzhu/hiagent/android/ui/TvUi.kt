package com.xiaoyuanzhu.hiagent.android.ui

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.scale
import androidx.compose.ui.focus.FocusDirection
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/**
 * The television's own measurements and controls.
 *
 * Two things are different enough from a handset to need saying once here rather
 * than at every call site: where the edge of the picture is, and how a control
 * says it is the one the remote is pointing at.
 */
object Tv {
    /**
     * The margin the picture may not be trusted inside.
     *
     * Televisions overscan — the panel crops the incoming frame and the amount is
     * the set's business, not the app's — so the platform's guidance is to keep
     * content inside 5% of each edge and let only background reach it. On the
     * 1920x1080 frame Android TV lays out as 960x540dp, 5% is these two numbers,
     * which are the same ones leanback's own browse padding uses.
     *
     * The face keeps its own content clear the same way, in CSS, off the `tv`
     * shape — this pair is only for the screens the shell draws.
     */
    val overscanHorizontal: Dp = 48.dp
    val overscanVertical: Dp = 27.dp

    /** The corner every TV control shares, large enough to read across a room. */
    val radius: Dp = 16.dp
}

/** Hold a screen's content inside the overscan margin. */
fun Modifier.overscan(): Modifier =
    padding(horizontal = Tv.overscanHorizontal, vertical = Tv.overscanVertical)

/**
 * The focus mark.
 *
 * On a handset the pointer *is* the cursor, so a control needs no permanent way
 * to say "you are here"; a 2dp `:focus-visible` outline is enough for the rare
 * keyboard user. Across a room, focus is the only cursor there is, and it has to
 * survive being looked at from three metres away — hence a ring plus a small
 * lift rather than an outline. Both animate, because a focus that jumps between
 * controls with no motion is hard to follow when the eye is not already on the
 * destination.
 */
private fun Modifier.focusMark(focused: Boolean, scale: Float, ring: androidx.compose.ui.graphics.Color): Modifier =
    this
        .scale(scale)
        .border(
            width = if (focused) 3.dp else 0.dp,
            color = if (focused) ring else androidx.compose.ui.graphics.Color.Transparent,
            shape = RoundedCornerShape(Tv.radius),
        )

/**
 * A button the remote can land on.
 *
 * D-pad centre needs no handling of its own: Compose's `clickable` — which
 * `Button` is built from — already treats centre and enter as a click on the
 * focused element.
 */
@Composable
fun TvButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    leading: ImageVector? = null,
    focusRequester: FocusRequester? = null,
) {
    var focused by remember { mutableStateOf(false) }
    val lift by animateFloatAsState(
        targetValue = if (focused) 1.04f else 1f,
        animationSpec = tween(140),
        label = "focus-lift",
    )

    Button(
        onClick = onClick,
        enabled = enabled,
        shape = RoundedCornerShape(Tv.radius),
        modifier = modifier
            .then(focusRequester?.let { Modifier.focusRequester(it) } ?: Modifier)
            .onFocusChanged { focused = it.isFocused }
            .focusMark(focused, lift, MaterialTheme.colorScheme.primary),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            modifier = Modifier.padding(vertical = 6.dp),
        ) {
            leading?.let { Icon(it, contentDescription = null, modifier = Modifier.size(22.dp)) }
            Text(text, style = MaterialTheme.typography.titleMedium)
        }
    }
}

/**
 * One line of typed input.
 *
 * Focus and editing are the same state in a Compose text field, so landing on one
 * with the D-pad raises the television's on-screen keyboard directly — there is
 * no second press to "enter" the field.
 *
 * Getting *out* of it takes explicit handling; see the key handler below for why
 * `singleLine` alone was not enough, which is the bug this comment used to assert
 * was impossible.
 */
@Composable
fun TvField(
    value: String,
    onValueChange: (String) -> Unit,
    label: String,
    modifier: Modifier = Modifier,
    placeholder: String? = null,
    /**
     * Fixed text pinned after the value, in the field's own trailing slot. The
     * add screen draws the zone here so `iloahz` reads as `iloahz.hi-agent.xyz`
     * while only the label was ever typed — which on a remote control is the
     * whole difference.
     */
    suffix: String? = null,
    keyboardOptions: KeyboardOptions = KeyboardOptions.Default,
    focusRequester: FocusRequester? = null,
    textStyle: androidx.compose.ui.text.TextStyle = MaterialTheme.typography.bodyLarge,
) {
    var focused by remember { mutableStateOf(false) }
    val focusManager = LocalFocusManager.current

    OutlinedTextField(
        value = value,
        onValueChange = onValueChange,
        label = { Text(label, style = MaterialTheme.typography.bodyLarge) },
        placeholder = placeholder?.let { { Text(it, style = MaterialTheme.typography.bodyLarge) } },
        suffix = suffix?.let {
            {
                Text(
                    it,
                    style = textStyle,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        },
        singleLine = true,
        textStyle = textStyle,
        keyboardOptions = keyboardOptions,
        shape = RoundedCornerShape(Tv.radius),
        modifier = modifier
            .then(focusRequester?.let { Modifier.focusRequester(it) } ?: Modifier)
            .onFocusChanged { focused = it.isFocused }
            // **Up and down have to leave the field, or the remote is stuck in it**
            // — and `singleLine` alone is not what guarantees that, which is what
            // this file used to claim. `BasicTextField` handles the arrow keys
            // itself, so the guarantee is worth stating rather than inheriting.
            // Taken in *preview*, before the field sees them; on one line there is
            // no vertical cursor movement to lose.
            //
            // **UNVERIFIED (2026-09-11).** In the TV emulator, D-pad down out of the
            // field did not reach the button and `TAB` did — but that emulator also
            // kept raising launcher and sign-in dialogs that took focus, so the
            // failure was never cleanly reproduced and this fix was never cleanly
            // observed to help. It is written from the mechanism, not from a
            // measurement. A real remote settles it; until then, treat "can you get
            // from the name field to the button" as an open question on this screen.
            .onPreviewKeyEvent { event ->
                if (event.type != KeyEventType.KeyDown) return@onPreviewKeyEvent false
                when (event.key) {
                    Key.DirectionDown -> focusManager.moveFocus(FocusDirection.Down)
                    Key.DirectionUp -> focusManager.moveFocus(FocusDirection.Up)
                    else -> false
                }
            }
            .focusMark(focused, 1f, MaterialTheme.colorScheme.primary),
    )
}
