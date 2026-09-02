package com.xiaoyuanzhu.hiagent.android.ui

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
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
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
import com.xiaoyuanzhu.hiagent.android.AppModel
import com.xiaoyuanzhu.hiagent.android.core.PairingRequest
import kotlinx.coroutines.launch

/**
 * Pairing, scan-first. The QR code is the path a core actually offers, so it
 * leads; typing an address and a code is the fallback underneath it, not the
 * form the screen opens as.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PairCoreSheet(
    model: AppModel,
    request: PairingRequest,
    onDismiss: () -> Unit,
) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()

    var baseUrl by rememberSaveable(request.id) { mutableStateOf(request.baseUrl) }
    var code by rememberSaveable(request.id) { mutableStateOf(request.code) }
    var label by rememberSaveable(request.id) { mutableStateOf(request.label) }
    var errorMessage by remember { mutableStateOf<String?>(null) }
    var isPairing by remember { mutableStateOf(false) }
    var showingScanner by remember { mutableStateOf(false) }

    LaunchedEffect(request.id) {
        if (request.opensScanner) showingScanner = true
    }

    val canPair = baseUrl.isNotBlank() && code.isNotBlank() && !isPairing

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
            Text("Pair a core", style = MaterialTheme.typography.titleMedium)

            CoreMark(size = 84.dp)

            Text(
                text = "Point this device at the pairing code your core is showing.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
                modifier = Modifier.widthIn(max = 320.dp),
            )

            Button(
                onClick = { showingScanner = true },
                modifier = Modifier.fillMaxWidth(),
                shape = RoundedCornerShape(14.dp),
            ) {
                Icon(Icons.Rounded.CameraAlt, contentDescription = null)
                Text("  Scan QR code")
            }

            Text(
                text = "or enter it by hand",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            OutlinedTextField(
                value = baseUrl,
                onValueChange = { baseUrl = it },
                label = { Text("Core address") },
                placeholder = { Text("https://hi-agent.xyz/ana") },
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
                label = { Text("Pairing code") },
                placeholder = { Text("one-time code") },
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

            errorMessage?.let { message ->
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

            Button(
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
                enabled = canPair,
                modifier = Modifier.fillMaxWidth(),
                shape = RoundedCornerShape(14.dp),
            ) {
                if (isPairing) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(16.dp),
                        strokeWidth = 2.dp,
                        color = Color.White,
                    )
                    Box(Modifier.size(8.dp))
                }
                Text(if (isPairing) "Pairing…" else "Pair")
            }
        }
    }

    if (showingScanner) {
        PairingQrScanner(
            onDismiss = { showingScanner = false },
            onScan = { scanned ->
                baseUrl = scanned.baseUrl
                code = scanned.code
                if (scanned.label.isNotEmpty()) label = scanned.label
                errorMessage = null
                showingScanner = false
            },
        )
    }
}
