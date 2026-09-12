package com.xiaoyuanzhu.hiagent.android.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.xiaoyuanzhu.hiagent.android.AppModel
import com.xiaoyuanzhu.hiagent.android.JoinInvitation
import com.xiaoyuanzhu.hiagent.android.core.AddAgentRequest
import com.xiaoyuanzhu.hiagent.android.core.CoreClient
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/**
 * Adding an agent from the sofa.
 *
 * This screen used to be the whole cost of this client. It said so itself: with no
 * camera, typing was not the fallback but the only way, and what had to be typed
 * was an address plus a 43-character one-time code, on an on-screen keyboard, with
 * a remote control. **Asking replaces that with a name** — six or so presses, and
 * then somebody says yes on a device that already has a keyboard.
 *
 * So the order here is not the handset's order with the camera removed. There is
 * no camera rung at all: a name, and underneath it the address-and-code form for a
 * self-hosted agent, which is now the rare case rather than the only one.
 *
 * Nothing about the wire changes for it. `POST /api/access/request` takes the same
 * body from a television that it takes from a phone, and the core is not told which
 * kind of device asked.
 */
@Composable
fun TvAddAgentScreen(
    model: AppModel,
    request: AddAgentRequest,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val scope = rememberCoroutineScope()

    var name by rememberSaveable(request.id) { mutableStateOf("") }
    var baseUrl by rememberSaveable(request.id) { mutableStateOf(request.baseUrl) }
    var code by rememberSaveable(request.id) { mutableStateOf(request.code) }
    var label by rememberSaveable(request.id) { mutableStateOf(request.label) }
    var errorMessage by remember { mutableStateOf<String?>(null) }
    var isWorking by remember { mutableStateOf(false) }
    // Arriving with an address in hand means this is the stage's "Add again", so
    // show the form it already has the answers for.
    var showingAddress by rememberSaveable(request.id) {
        mutableStateOf(request.baseUrl.isNotEmpty())
    }
    var invitation by remember { mutableStateOf<JoinInvitation?>(null) }

    val nameField = remember { FocusRequester() }
    val askButton = remember { FocusRequester() }
    LaunchedEffect(request.id, invitation) {
        if (invitation == null) nameField.requestFocus()
    }

    // Leaving the screen cancels the poll; the request expires on its own at the
    // agent, so there is nothing to tear down.
    LaunchedEffect(invitation) {
        val waiting = invitation ?: return@LaunchedEffect
        try {
            model.join(waiting)
            onDismiss()
        } catch (_: CancellationException) {
            // Left the screen.
        } catch (e: Exception) {
            errorMessage = e.message ?: "That did not work."
            invitation = null
        }
    }

    BackHandler(enabled = !isWorking) {
        if (invitation != null) invitation = null else onDismiss()
    }

    Box(modifier.hiCanvas(), contentAlignment = Alignment.Center) {
        Column(
            modifier = Modifier
                .overscan()
                .verticalScroll(rememberScrollState())
                .widthIn(max = 640.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(18.dp),
        ) {
            val waiting = invitation
            if (waiting != null) {
                TvWaitingOn(waiting, errorMessage) { invitation = null }
            } else {
                Text("Add a remote hi-agent", style = MaterialTheme.typography.headlineMedium)

                Text(
                    text = "Say which agent this is. It will ask to be let in, and " +
                        "you approve it on your phone or computer — nothing long to " +
                        "type here.",
                    style = MaterialTheme.typography.titleMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )

                // The zone sits in the field's own suffix slot, so the television
                // shows `iloahz.hi-agent.xyz` while only `iloahz` was ever typed —
                // and it disappears the moment what is typed stops being a label,
                // because `addressForName` then stops appending it. A box reading
                // `http://10.0.2.2:12392.hi-agent.xyz` describes a request nobody
                // is about to make.
                TvField(
                    value = name,
                    onValueChange = { name = it },
                    label = "Your agent's name",
                    placeholder = "your agent",
                    suffix = ".${CoreClient.DEFAULT_ZONE}".takeIf { isBareLabel(name) },
                    focusRequester = nameField,
                    // Down goes to the *primary* action, not to whichever button is
                    // nearest the field's centre. Compose's 2D focus search picks by
                    // geometry, and the field spans the row, so unaided it lands on
                    // the middle button — pressing down from a name and landing on
                    // "Hide address" is a remote doing something nobody asked for.
                    modifier = Modifier
                        .fillMaxWidth()
                        .focusProperties { down = askButton },
                    textStyle = MaterialTheme.typography.bodyLarge.copy(
                        fontFamily = FontFamily.Monospace,
                    ),
                    keyboardOptions = KeyboardOptions(
                        keyboardType = KeyboardType.Uri,
                        autoCorrectEnabled = false,
                        imeAction = ImeAction.Go,
                    ),
                )

                errorMessage?.let { message -> TvErrorLine(message) }

                Row(
                    modifier = Modifier.padding(top = 6.dp),
                    horizontalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    TvButton(
                        text = if (isWorking) "Asking…" else "Ask to be let in",
                        focusRequester = askButton,
                        enabled = name.isNotBlank() && !isWorking,
                        onClick = {
                            scope.launch {
                                isWorking = true
                                errorMessage = null
                                try {
                                    invitation = model.askToJoin(name)
                                } catch (e: Exception) {
                                    errorMessage = e.message ?: "That did not work."
                                } finally {
                                    isWorking = false
                                }
                            }
                        },
                    )
                    TvButton(
                        text = if (showingAddress) "Hide address" else "Use a full address",
                        enabled = !isWorking,
                        onClick = { showingAddress = !showingAddress },
                    )
                    TvButton(text = "Cancel", enabled = !isWorking, onClick = onDismiss)
                }

                if (showingAddress) {
                    TvField(
                        value = baseUrl,
                        onValueChange = { baseUrl = it },
                        label = "Address",
                        placeholder = "https://ana.hi-agent.xyz",
                        modifier = Modifier.fillMaxWidth(),
                        keyboardOptions = KeyboardOptions(
                            keyboardType = KeyboardType.Uri,
                            autoCorrectEnabled = false,
                            imeAction = ImeAction.Next,
                        ),
                    )

                    TvField(
                        value = code,
                        onValueChange = { code = it },
                        label = "One-time code",
                        placeholder = "the code it is showing",
                        modifier = Modifier.fillMaxWidth(),
                        textStyle = MaterialTheme.typography.bodyLarge.copy(
                            fontFamily = FontFamily.Monospace,
                        ),
                        keyboardOptions = KeyboardOptions(
                            keyboardType = KeyboardType.Password,
                            autoCorrectEnabled = false,
                            imeAction = ImeAction.Next,
                        ),
                    )

                    TvField(
                        value = label,
                        onValueChange = { label = it },
                        label = "Name (optional)",
                        placeholder = "only shown on this television",
                        modifier = Modifier.fillMaxWidth(),
                        keyboardOptions = KeyboardOptions(
                            capitalization = KeyboardCapitalization.Words,
                            imeAction = ImeAction.Done,
                        ),
                    )

                    TvButton(
                        text = if (isWorking) "Adding…" else "Add",
                        enabled = baseUrl.isNotBlank() && code.isNotBlank() && !isWorking,
                        onClick = {
                            scope.launch {
                                isWorking = true
                                errorMessage = null
                                try {
                                    model.add(baseUrl, code, label)
                                    onDismiss()
                                } catch (e: Exception) {
                                    errorMessage = e.message ?: "That did not work."
                                } finally {
                                    isWorking = false
                                }
                            }
                        },
                    )
                }
            }
        }
    }
}

/**
 * The wait, at across-the-room size. The code is the biggest thing on the
 * television because the person reading it is on the sofa and the device they are
 * about to approve it on is in their hand.
 */
@Composable
private fun TvWaitingOn(
    invitation: JoinInvitation,
    errorMessage: String?,
    onCancel: () -> Unit,
) {
    val cancel = remember { FocusRequester() }
    val keyboard = LocalSoftwareKeyboardController.current
    LaunchedEffect(Unit) {
        // The name field raised the on-screen keyboard, and nothing takes it away
        // when this screen replaces that one — it sits over the very line telling
        // the person what to go and do. There is nothing to type here.
        keyboard?.hide()
        cancel.requestFocus()
    }

    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(20.dp),
    ) {
        Text("Waiting to be let in", style = MaterialTheme.typography.headlineMedium)

        Text(
            text = invitation.code,
            style = MaterialTheme.typography.displayLarge.copy(
                fontFamily = FontFamily.Monospace,
                letterSpacing = 12.sp,
            ),
        )

        Text(
            text = "Open Reach on ${invitation.label} and let this television in. " +
                "Check the code matches.",
            style = MaterialTheme.typography.titleMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        errorMessage?.let { message -> TvErrorLine(message) }

        TvButton(text = "Cancel", onClick = onCancel, focusRequester = cancel)
    }
}

@Composable
private fun TvErrorLine(message: String) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.Top,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Icon(
            Icons.Rounded.Warning,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.error,
            modifier = Modifier.size(22.dp),
        )
        Text(
            text = message,
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.error,
        )
    }
}

/**
 * Whether what has been typed is still a bare label — the only case in which
 * [CoreClient.addressForName] appends the zone, and therefore the only case in
 * which the zone should be on screen.
 */
private fun isBareLabel(typed: String): Boolean {
    val bare = typed.trim().removePrefix("@")
    return !bare.contains('.') && !bare.contains("://") && !bare.contains('/')
}
