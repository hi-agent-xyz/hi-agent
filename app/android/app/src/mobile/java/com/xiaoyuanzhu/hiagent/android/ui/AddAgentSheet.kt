package com.xiaoyuanzhu.hiagent.android.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.CameraAlt
import androidx.compose.material.icons.rounded.ExpandMore
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.xiaoyuanzhu.hiagent.android.AppModel
import com.xiaoyuanzhu.hiagent.android.JoinInvitation
import com.xiaoyuanzhu.hiagent.android.core.AddAgentRequest
import com.xiaoyuanzhu.hiagent.android.core.CoreClient
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/**
 * Adding a remote agent, name-first — the same three states the iOS sheet has, in
 * this toolkit's own vocabulary.
 *
 * **The name leads because it is the only path that works from anywhere.** A QR
 * scan only helps when you are standing at the machine showing the code, and if
 * you are standing there you are standing where the Approve button is. Typing a
 * full address and a one-time code is still here, folded away, because a
 * self-hosted agent has no name in the default zone — and because the stage's
 * "Add again" arrives with an address already filled in.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AddAgentSheet(
    model: AppModel,
    request: AddAgentRequest,
    onDismiss: () -> Unit,
) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()

    var name by rememberSaveable(request.id) { mutableStateOf("") }
    var baseUrl by rememberSaveable(request.id) { mutableStateOf(request.baseUrl) }
    var code by rememberSaveable(request.id) { mutableStateOf(request.code) }
    var label by rememberSaveable(request.id) { mutableStateOf(request.label) }
    var errorMessage by remember { mutableStateOf<String?>(null) }
    var isWorking by remember { mutableStateOf(false) }
    var showingScanner by remember { mutableStateOf(false) }
    // An "Add again" from the stage arrives knowing the address, so open on the
    // card that holds it rather than on a name field it already has the answer to.
    var showingAddress by rememberSaveable(request.id) {
        mutableStateOf(request.baseUrl.isNotEmpty())
    }
    var invitation by remember { mutableStateOf<JoinInvitation?>(null) }

    LaunchedEffect(request.id) {
        if (request.opensScanner) showingScanner = true
    }

    // The wait belongs to this screen: leaving it cancels the poll, and the request
    // expires on its own at the agent.
    LaunchedEffect(invitation) {
        val waiting = invitation ?: return@LaunchedEffect
        try {
            model.join(waiting)
            onDismiss()
        } catch (_: CancellationException) {
            // Left the screen. Nothing to say.
        } catch (e: Exception) {
            errorMessage = e.message ?: "That did not work."
            invitation = null
        }
    }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = MaterialTheme.colorScheme.surface,
        shape = RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp),
    ) {
        Column(
            Modifier
                .verticalScroll(rememberScrollState())
                .imePadding()
                .navigationBarsPadding()
                .padding(horizontal = Theme.gutter)
                .padding(bottom = 24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(18.dp),
        ) {
            Text("Add a remote hi-agent", style = MaterialTheme.typography.titleMedium)

            val waiting = invitation
            if (waiting != null) {
                WaitingOn(waiting, errorMessage) { invitation = null }
            } else {
                CoreMark(size = 84.dp)

                Text(
                    text = "Say which agent this is. It will ask to be let in, " +
                        "and you approve it there.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                    modifier = Modifier.widthIn(max = 320.dp),
                )

                // The zone rides in the field's own trailing slot, so typing
                // `iloahz` reads as `iloahz.hi-agent.xyz` and no sentence has to be
                // spent explaining what a name is.
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it },
                    placeholder = { Text("your agent") },
                    // Shown only while what is typed is still a bare label, which
                    // is the only case `addressForName` appends the zone in. A box
                    // reading `example.com.hi-agent.xyz` describes a request nobody
                    // is about to make.
                    suffix = if (isBareLabel(name)) {
                        {
                            Text(
                                ".${CoreClient.DEFAULT_ZONE}",
                                fontFamily = FontFamily.Monospace,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    } else {
                        null
                    },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                    textStyle = MaterialTheme.typography.bodyLarge.copy(
                        fontFamily = FontFamily.Monospace,
                    ),
                    keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(
                        keyboardType = KeyboardType.Uri,
                        autoCorrectEnabled = false,
                        imeAction = ImeAction.Go,
                    ),
                )

                Button(
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
                    enabled = name.isNotBlank() && !isWorking,
                    modifier = Modifier.fillMaxWidth(),
                    shape = RoundedCornerShape(14.dp),
                ) {
                    if (isWorking) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(16.dp),
                            strokeWidth = 2.dp,
                            color = Color.White,
                        )
                        Box(Modifier.size(8.dp))
                    }
                    Text(if (isWorking) "Asking…" else "Ask to be let in")
                }

                TextButton(onClick = { showingScanner = true }) {
                    Icon(Icons.Rounded.CameraAlt, contentDescription = null)
                    Text("  Scan a QR code instead")
                }

                TextButton(onClick = { showingAddress = !showingAddress }) {
                    Text("Use a full address")
                    Icon(Icons.Rounded.ExpandMore, contentDescription = null)
                }

                AnimatedVisibility(visible = showingAddress) {
                    Column(
                        verticalArrangement = Arrangement.spacedBy(14.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        OutlinedTextField(
                            value = baseUrl,
                            onValueChange = { baseUrl = it },
                            label = { Text("Address") },
                            placeholder = { Text("https://ana.hi-agent.xyz") },
                            singleLine = true,
                            modifier = Modifier.fillMaxWidth(),
                            keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(
                                keyboardType = KeyboardType.Uri,
                                autoCorrectEnabled = false,
                                imeAction = ImeAction.Next,
                            ),
                        )

                        OutlinedTextField(
                            value = code,
                            onValueChange = { code = it },
                            label = { Text("One-time code") },
                            placeholder = { Text("the code it is showing") },
                            singleLine = true,
                            modifier = Modifier.fillMaxWidth(),
                            textStyle = MaterialTheme.typography.bodyLarge.copy(
                                fontFamily = FontFamily.Monospace,
                            ),
                            keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(
                                keyboardType = KeyboardType.Password,
                                autoCorrectEnabled = false,
                                imeAction = ImeAction.Next,
                            ),
                        )

                        OutlinedTextField(
                            value = label,
                            onValueChange = { label = it },
                            label = { Text("Name (optional)") },
                            placeholder = { Text("only shown on this device") },
                            singleLine = true,
                            modifier = Modifier.fillMaxWidth(),
                            keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(
                                capitalization = KeyboardCapitalization.Words,
                                imeAction = ImeAction.Done,
                            ),
                        )

                        OutlinedButton(
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
                            enabled = baseUrl.isNotBlank() && code.isNotBlank() && !isWorking,
                            modifier = Modifier.fillMaxWidth(),
                            shape = RoundedCornerShape(14.dp),
                        ) {
                            Text("Add")
                        }
                    }
                }

                errorMessage?.let { message -> ErrorLine(message) }
            }
        }
    }

    if (showingScanner) {
        AgentQrScanner(
            onDismiss = { showingScanner = false },
            onScan = { scanned ->
                baseUrl = scanned.baseUrl
                code = scanned.code
                if (scanned.label.isNotEmpty()) label = scanned.label
                showingAddress = true
                errorMessage = null
                showingScanner = false
            },
        )
    }
}

/**
 * The wait. One number, big, and the sentence that says who has to act — there is
 * nothing for the person holding this device to do but hold it up.
 */
@Composable
private fun WaitingOn(
    invitation: JoinInvitation,
    errorMessage: String?,
    onCancel: () -> Unit,
) {
    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Text(
            text = invitation.code,
            style = MaterialTheme.typography.displayMedium.copy(
                fontFamily = FontFamily.Monospace,
                letterSpacing = 6.sp,
            ),
        )

        Text(
            text = "Approve this on ${invitation.label} to finish.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
            modifier = Modifier.widthIn(max = 320.dp),
        )

        CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)

        errorMessage?.let { message -> ErrorLine(message) }

        TextButton(onClick = onCancel) { Text("Cancel") }
    }
}

@Composable
private fun ErrorLine(message: String) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(top = 2.dp),
        verticalAlignment = Alignment.Top,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Icon(
            Icons.Rounded.Warning,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.error,
            modifier = Modifier.size(18.dp),
        )
        Text(
            text = message,
            style = MaterialTheme.typography.bodySmall,
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
