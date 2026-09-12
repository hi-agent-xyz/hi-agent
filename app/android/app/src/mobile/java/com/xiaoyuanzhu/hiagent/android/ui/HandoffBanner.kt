package com.xiaoyuanzhu.hiagent.android.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.WarningAmber
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.xiaoyuanzhu.hiagent.android.core.HandoffState
import kotlinx.coroutines.delay

/**
 * What happened to the thing you just shared.
 *
 * A share arrives over whatever the person was doing and its whole result is a
 * message in a conversation that may still be painting. Without this, "sent" and
 * "this device isn't paired with anything" look identical: the app opened, and
 * nothing visibly happened.
 *
 * It says its piece and goes. Only a failure stays, because only a failure is
 * something to act on — and there is no Try again, because unlike the iOS queue there
 * is nothing kept to retry from: the way to send it again is to share it again.
 */
@Composable
fun HandoffBanner(
    state: HandoffState?,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    LaunchedEffect(state) {
        if (state is HandoffState.Sent) {
            // Long enough to read six words while the face is still painting.
            delay(2400)
            onDismiss()
        }
    }

    AnimatedVisibility(
        visible = state != null,
        enter = slideInVertically { it } + fadeIn(),
        exit = slideOutVertically { it } + fadeOut(),
        modifier = modifier,
    ) {
        Surface(
            shape = RoundedCornerShape(16.dp),
            tonalElevation = 3.dp,
            shadowElevation = 6.dp,
            modifier = Modifier
                .navigationBarsPadding()
                .padding(horizontal = Theme.gutter, vertical = 14.dp),
        ) {
            Column(modifier = Modifier.padding(horizontal = 14.dp, vertical = 12.dp)) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    when (state) {
                        is HandoffState.Sending -> {
                            CircularProgressIndicator(
                                modifier = Modifier.size(18.dp),
                                strokeWidth = 2.dp,
                            )
                            Text(
                                text = if (state.count > 1) {
                                    "Sending ${state.count} things…"
                                } else {
                                    "Sending…"
                                },
                                style = MaterialTheme.typography.bodyMedium,
                            )
                        }

                        is HandoffState.Sent -> {
                            Icon(
                                Icons.Rounded.CheckCircle,
                                contentDescription = null,
                                modifier = Modifier.size(18.dp),
                            )
                            Text(
                                text = if (state.count > 1) {
                                    "Sent ${state.count} to ${state.coreLabel}"
                                } else {
                                    "Sent to ${state.coreLabel}"
                                },
                                style = MaterialTheme.typography.bodyMedium,
                            )
                        }

                        is HandoffState.Failed -> {
                            Icon(
                                Icons.Rounded.WarningAmber,
                                contentDescription = null,
                                modifier = Modifier.size(18.dp),
                            )
                            Text(state.reason, style = MaterialTheme.typography.bodyMedium)
                        }

                        null -> Unit
                    }
                }

                if (state is HandoffState.Failed) {
                    TextButton(
                        onClick = onDismiss,
                        modifier = Modifier.align(Alignment.End),
                    ) {
                        Text("Dismiss")
                    }
                }
            }
        }
    }
}
