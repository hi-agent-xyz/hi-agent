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
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import com.xiaoyuanzhu.hiagent.android.AppModel
import com.xiaoyuanzhu.hiagent.android.core.PairingRequest
import kotlinx.coroutines.launch

/**
 * Pairing, typed.
 *
 * The handset leads with the camera and keeps typing as the fallback underneath;
 * here there is no camera, so the fallback is the whole screen. That is the one
 * real cost of this client, and it is paid once per core: an address and a
 * one-time code entered on the television's own keyboard, after which the roster
 * remembers the core and the Keystore remembers the credential.
 *
 * Nothing about the wire changes for it. `POST /api/session` takes the same code
 * from a television that it takes from a phone, and the core is not told which
 * kind of device spent it.
 */
@Composable
fun TvPairScreen(
    model: AppModel,
    request: PairingRequest,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val scope = rememberCoroutineScope()

    var baseUrl by rememberSaveable(request.id) { mutableStateOf(request.baseUrl) }
    var code by rememberSaveable(request.id) { mutableStateOf(request.code) }
    var label by rememberSaveable(request.id) { mutableStateOf(request.label) }
    var errorMessage by remember { mutableStateOf<String?>(null) }
    var isPairing by remember { mutableStateOf(false) }

    val addressField = remember { FocusRequester() }
    LaunchedEffect(request.id) { addressField.requestFocus() }

    BackHandler(enabled = !isPairing) { onDismiss() }

    val canPair = baseUrl.isNotBlank() && code.isNotBlank() && !isPairing

    Box(modifier.hiCanvas(), contentAlignment = Alignment.Center) {
        Column(
            modifier = Modifier
                .overscan()
                .verticalScroll(rememberScrollState())
                .widthIn(max = 640.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(18.dp),
        ) {
            Text("Pair a core", style = MaterialTheme.typography.headlineMedium)

            Text(
                text = "Ask your core for a pairing code, then enter its address " +
                    "and the code here.",
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            TvField(
                value = baseUrl,
                onValueChange = { baseUrl = it },
                label = "Core address",
                placeholder = "https://hi-agent.xyz/ana",
                focusRequester = addressField,
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
                label = "Pairing code",
                placeholder = "one-time code",
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

            errorMessage?.let { message ->
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

            Row(
                modifier = Modifier.padding(top = 6.dp),
                horizontalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                TvButton(
                    text = if (isPairing) "Pairing…" else "Pair",
                    enabled = canPair,
                    onClick = {
                        scope.launch {
                            isPairing = true
                            errorMessage = null
                            try {
                                model.pair(baseUrl, code, label)
                                onDismiss()
                            } catch (e: Exception) {
                                errorMessage = e.message ?: "Pairing did not work."
                            } finally {
                                isPairing = false
                            }
                        }
                    },
                )
                TvButton(text = "Cancel", enabled = !isPairing, onClick = onDismiss)
            }
        }
    }
}
